// Persisted "Viewed" marks for a PR's Code tab. Each mark records the file's
// signature at the time it was ticked, so a push that changes the file drops
// the mark (GitHub's "viewed until it changes" rule) while an untouched file
// stays viewed across reloads, tabs and app restarts. Storage is best-effort:
// a blocked/full localStorage just means marks don't persist.
//
// A signature is the file's stat (`status:added:deleted`) plus, once its hunks
// were loaded when it was ticked, `#<hash of the patch text>`. The stat alone
// missed a push that rewrote lines without changing the counts. A summary diff
// has no hunks yet, so a hashed mark is accepted on its stat until the file's
// hunks arrive and is re-checked then (`staleViewed`). A stat-only mark (older
// builds, or ticked before the hunks loaded) still matches on its stat, so an
// upgrade wipes no marks.
import type { FileDiff } from '../../lib/api/types';

/** Change signature of one file's stat — the part every mark carries. */
export function viewSig(f: Pick<FileDiff, 'status' | 'added' | 'deleted'>): string {
  return `${f.status ?? ''}:${f.added ?? ''}:${f.deleted ?? ''}`;
}

const hashCache = new WeakMap<FileDiff, string | null>();
/** FNV-1a (32-bit, hex) of the patch text, or null when the hunks are not at
 *  hand (summary mode, too large, binary). Cached per file object. */
export function patchHash(f: FileDiff): string | null {
  const hit = hashCache.get(f);
  if (hit !== undefined) return hit;
  let out: string | null = null;
  if (!f.is_binary && !f.too_large && f.hunks.length > 0) {
    let h = 0x811c9dc5;
    const mix = (s: string): void => {
      for (let i = 0; i < s.length; i++) {
        h ^= s.charCodeAt(i);
        h = Math.imul(h, 0x01000193);
      }
    };
    for (const hunk of f.hunks) {
      mix(hunk.header);
      for (const l of hunk.lines) mix(`\n${l.origin === 'add' ? '+' : l.origin === 'del' ? '-' : ' '}${l.content}`);
    }
    out = (h >>> 0).toString(16).padStart(8, '0');
  }
  hashCache.set(f, out);
  return out;
}

/** The signature a mark is stored with: stat, plus the patch hash of
 *  `loaded` (the file with its hunks) when there is one. */
export function markSig(f: FileDiff, loaded?: FileDiff | null): string {
  const h = loaded ? patchHash(loaded) : null;
  return h ? `${viewSig(f)}#${h}` : viewSig(f);
}

/** Does the stored mark `stored` still hold for `f`? `loaded` is the file
 *  with its hunks when they are at hand; without them a hashed mark is
 *  provisionally accepted on its stat. */
export function markHolds(stored: string, f: FileDiff, loaded?: FileDiff | null): boolean {
  const at = stored.indexOf('#');
  const stat = at < 0 ? stored : stored.slice(0, at);
  if (stat !== viewSig(f)) return false;
  if (at < 0) return true;
  const now = loaded ? patchHash(loaded) : null;
  return now === null || now === stored.slice(at + 1);
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

type Loaded = ReadonlyMap<string, FileDiff> | undefined;

/** The stored marks of `key` that still match the files in `files`. */
export function loadViewed(key: string, files: readonly FileDiff[], loaded?: Loaded): Set<string> {
  const stored = read(key);
  const out = new Set<string>();
  for (const f of files) {
    const sig = stored[f.path];
    if (typeof sig === 'string' && markHolds(sig, f, loaded?.get(f.path))) out.add(f.path);
  }
  return out;
}

/** Paths in `viewed` whose stored mark the just-loaded hunks in `landed`
 *  contradict (same stat, different patch: a push that kept the counts). */
export function staleViewed(key: string, files: readonly FileDiff[], viewed: ReadonlySet<string>, landed: ReadonlyMap<string, FileDiff>): string[] {
  const out: string[] = [];
  if (landed.size === 0 || viewed.size === 0) return out;
  const stored = read(key);
  for (const f of files) {
    const fd = landed.get(f.path);
    const sig = stored[f.path];
    if (fd && viewed.has(f.path) && typeof sig === 'string' && !markHolds(sig, f, fd)) out.push(f.path);
  }
  return out;
}

/** Persist `viewed` for the files in `files`, pinned to each file's current
 *  signature. MERGES into the stored record: a mark for a path this view
 *  does not show (another commit's view, hidden test files) is kept. A
 *  still-valid hashed mark is kept as is when the hunks are not at hand now,
 *  so re-saving from a summary view never weakens it to a stat-only mark. */
export function saveViewed(key: string, files: readonly FileDiff[], viewed: ReadonlySet<string>, loaded?: Loaded): void {
  try {
    const rec = read(key);
    for (const f of files) {
      if (!viewed.has(f.path)) {
        delete rec[f.path];
        continue;
      }
      const fd = loaded?.get(f.path);
      const prev = rec[f.path];
      const next = markSig(f, fd);
      rec[f.path] = typeof prev === 'string' && prev.includes('#') && !next.includes('#') && markHolds(prev, f, fd) ? prev : next;
    }
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
