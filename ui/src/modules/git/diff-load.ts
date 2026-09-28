// Lazy diff loading (docs/contracts/api.md, `/diff` + PR diff): the viewer
// paints a `summary=true` file list first and fetches one file's hunks only
// when that file is expanded AND near the viewport. Every fetch takes an
// AbortSignal. The PR diff is shared between the Files tab (PrDetail) and the
// Review tab's comment snippets (ReviewPanel) through one short-lived cache,
// so a PR visit downloads it once instead of twice.
import { api } from '../../lib/api/client';
import type { DiffResp, FileDiff } from '../../lib/api/types';

/** Fetch one file's hunks. `full` bypasses the per-file cap ("Load anyway").
 *  Resolves null when the server has no diff for that path. */
export type DiffFileLoader = (
  file: FileDiff,
  opts: { full: boolean; signal: AbortSignal },
) => Promise<FileDiff | null>;

function fileQuery(file: FileDiff, full: boolean): string {
  let q = `path=${encodeURIComponent(file.path)}`;
  if (file.old_path && file.old_path !== file.path) q += `&old_path=${encodeURIComponent(file.old_path)}`;
  if (full) q += '&full=true';
  return q;
}

/** The entry for `path` in a diff response. A type change (file ↔ symlink)
 *  is ONE summary row but TWO patch entries for the same path — a delete and
 *  an add; they are merged so the added half isn't silently dropped. */
export function fileFor(resp: DiffResp, path: string): FileDiff | null {
  const hits = resp.files.filter((f) => f.path === path);
  if (hits.length <= 1) return hits[0] ?? null;
  const main = hits.find((f) => f.status !== 'deleted') ?? hits[0];
  const sum = (k: 'added' | 'deleted') =>
    hits.every((h) => h[k] != null) ? hits.reduce((n, h) => n + (h[k] ?? 0), 0) : main[k];
  return {
    ...main,
    hunks: hits.flatMap((h) => h.hunks),
    too_large: hits.some((h) => h.too_large) || main.too_large,
    hunks_omitted: hits.some((h) => h.hunks_omitted) || main.hunks_omitted,
    added: sum('added'),
    deleted: sum('deleted'),
  };
}

function pick(resp: DiffResp, file: FileDiff): FileDiff | null {
  return fileFor(resp, file.path) ?? resp.files[0] ?? null;
}

/** Loader for a local repo diff target (`worktree` | `staged` | `working` |
 *  `commit:<sha>` | `range:<a>..<b>`). */
export function repoDiffFileLoader(repoId: string, target: string): DiffFileLoader {
  return async (file, { full, signal }) => {
    const r = await api.get<DiffResp>(
      `/repos/${repoId}/diff?target=${encodeURIComponent(target)}&${fileQuery(file, full)}`,
      signal,
    );
    return pick(r, file);
  };
}

// ── Shared PR diff cache ──────────────────────────────────────────────────
// Keyed by repo + PR number with a short TTL, and bound to the PR's head
// commit (`PrSummary.head_sha`) once a caller knows it: a different head —
// a push — starts a new entry, and every request of an entry carries the
// `rev=` it was created with, so the daemon's memo is re-keyed too (a stale
// pre-push diff is never served, even inside the TTL).
// A shared fetch runs on its own controller: a caller's abort only detaches
// that caller, and the fetch itself is aborted when nobody is waiting on it.
const PR_TTL_MS = 60_000;
// Bounds (r3-05-02): the TTL only decided whether a REVISIT reused an entry —
// nothing ever deleted one, so every PR opened in a webview's life stayed
// resident (15–30 MB for a fully browsed 100k-line PR, 50–200 MB after a day
// of reviews). Entries are now evicted on write: expired ones, then the
// least-recently used past PR_MAX_ENTRIES or an estimated PR_MAX_BYTES, never
// one a caller is still waiting on. A revisit after eviction costs one fetch
// (the daemon's own `pr_diffs` memo absorbs it).
const PR_MAX_ENTRIES = 8;
const PR_MAX_BYTES = 48 * 1024 * 1024;

interface Shared<T> {
  promise: Promise<T>;
  ctl: AbortController;
  waiters: number;
  settled: boolean;
}
interface PrEntry {
  at: number;
  /** The head this entry's data belongs to ('' = not known yet). */
  head: string;
  /** Query suffix every request of this entry sends (`&rev=<head at creation>`). */
  rev: string;
  summary: Shared<DiffResp> | null;
  files: Map<string, Shared<FileDiff | null>>;
  /** Estimated JS-heap bytes of the settled data (see diffBytes). */
  bytes: number;
}
/** Insertion order = recency (a touch re-inserts), so the first key is the LRU. */
const prCache = new Map<string, PrEntry>();

/** Rough heap size of a file's hunks: UTF-16 text plus per-object overhead. */
export function fileDiffBytes(f: FileDiff | null | undefined): number {
  if (!f) return 64;
  let n = 256 + (f.path.length + (f.old_path?.length ?? 0)) * 2;
  for (const h of f.hunks ?? []) {
    n += 96 + h.header.length * 2;
    for (const l of h.lines) n += 56 + l.content.length * 2;
  }
  return n;
}

function busy(e: PrEntry): boolean {
  if (e.summary && e.summary.waiters > 0) return true;
  for (const sh of e.files.values()) if (sh.waiters > 0) return true;
  return false;
}

/** Evict expired entries, then LRU ones past the count / byte budget. The
 *  entry just touched (`keep`) and entries with waiters stay. */
function evictPrCache(keep: string): void {
  const now = Date.now();
  for (const [k, e] of prCache) {
    if (k !== keep && now - e.at > PR_TTL_MS && !busy(e)) prCache.delete(k);
  }
  let total = 0;
  for (const e of prCache.values()) total += e.bytes;
  for (const [k, e] of prCache) {
    if (prCache.size <= PR_MAX_ENTRIES && total <= PR_MAX_BYTES) break;
    if (k === keep || busy(e)) continue;
    prCache.delete(k);
    total -= e.bytes;
  }
}

/** Charge settled data to its entry (only while it is still the live one). */
function charge(key: string, e: PrEntry, bytes: number): void {
  if (prCache.get(key) !== e) return;
  e.bytes += bytes;
  evictPrCache(key);
}

/** Test/diagnostic view of the cache: [key, bytes] in LRU → MRU order. */
export function prCacheSnapshot(): [string, number][] {
  return [...prCache].map(([k, e]) => [k, e.bytes]);
}

/** The live entry for a PR. `head` (when the caller knows it) replaces an
 *  entry built for a different head; an entry that didn't know its head yet
 *  adopts it — it was fetched moments ago, alongside the PR detail. */
function prEntry(repoId: string, num: number, head?: string | null): PrEntry {
  const key = `${repoId}#${num}`;
  let e = prCache.get(key);
  const expired = !e || Date.now() - e.at > PR_TTL_MS;
  if (!e || expired || (head && e.head && e.head !== head)) {
    const h = head || e?.head || '';
    e = { at: Date.now(), head: h, rev: h ? `&rev=${encodeURIComponent(h)}` : '', summary: null, files: new Map(), bytes: 0 };
    prCache.delete(key);
    prCache.set(key, e);
  } else {
    if (head && !e.head) e.head = head;
    // Touch: move to the most-recently-used end.
    prCache.delete(key);
    prCache.set(key, e);
  }
  evictPrCache(key);
  return e;
}

function share<T>(run: (signal: AbortSignal) => Promise<T>): Shared<T> {
  const ctl = new AbortController();
  const s: Shared<T> = { promise: undefined as unknown as Promise<T>, ctl, waiters: 0, settled: false };
  s.promise = run(ctl.signal).finally(() => {
    s.settled = true;
  });
  return s;
}

/** Await a shared fetch on behalf of one caller. On the caller's abort it
 *  rejects with AbortError; the last waiter to leave aborts the fetch and
 *  `drop` evicts it so the next caller starts fresh. */
function join<T>(s: Shared<T>, signal: AbortSignal | undefined, drop: () => void): Promise<T> {
  s.waiters++;
  if (!signal) {
    return s.promise.finally(() => s.waiters--);
  }
  return new Promise<T>((resolve, reject) => {
    const onAbort = () => {
      s.waiters--;
      if (s.waiters <= 0 && !s.settled) {
        s.ctl.abort();
        drop();
      }
      reject(new DOMException('Aborted', 'AbortError'));
    };
    if (signal.aborted) {
      onAbort();
      return;
    }
    signal.addEventListener('abort', onAbort, { once: true });
    s.promise.then(
      (v) => {
        signal.removeEventListener('abort', onAbort);
        s.waiters--;
        resolve(v);
      },
      (e) => {
        signal.removeEventListener('abort', onAbort);
        s.waiters--;
        reject(e);
      },
    );
  });
}

/** The PR's file list (summary mode). An older daemon that ignores
 *  `summary=true` returns full hunks — those files are cached as loaded. */
export function prDiffSummary(
  repoId: string,
  num: number,
  signal?: AbortSignal,
  head?: string | null,
): Promise<DiffResp> {
  const e = prEntry(repoId, num, head);
  if (!e.summary) {
    const sh = share((sig) => api.get<DiffResp>(`/repos/${repoId}/prs/${num}/diff?summary=true${e.rev}`, sig));
    e.summary = sh;
    const key = `${repoId}#${num}`;
    sh.promise.then(
      (resp) => {
        let bytes = 0;
        for (const f of resp.files) bytes += fileDiffBytes(f);
        charge(key, e, bytes);
        for (const f of resp.files) {
          if (f.hunks_omitted || f.too_large || e.files.has(f.path)) continue;
          const done: Shared<FileDiff | null> = {
            promise: Promise.resolve(f),
            ctl: new AbortController(),
            waiters: 0,
            settled: true,
          };
          e.files.set(f.path, done);
        }
      },
      () => {
        if (e.summary === sh) e.summary = null; // a failure is retried, not cached
      },
    );
  }
  const sh = e.summary;
  return join(sh, signal, () => {
    if (e.summary === sh) e.summary = null;
  });
}

/** One PR file's hunks (per-file route), shared across panels. */
export function prDiffFile(
  repoId: string,
  num: number,
  file: FileDiff,
  full: boolean,
  signal?: AbortSignal,
): Promise<FileDiff | null> {
  const e = prEntry(repoId, num);
  const key = full ? `${file.path}\u0000full` : file.path;
  let sh = e.files.get(key);
  if (!sh) {
    const created = share((sig) =>
      api.get<DiffResp>(`/repos/${repoId}/prs/${num}/diff?${fileQuery(file, full)}${e.rev}`, sig).then((r) => pick(r, file)),
    );
    created.promise.then(
      (f) => charge(`${repoId}#${num}`, e, fileDiffBytes(f)),
      () => {
        if (e.files.get(key) === created) e.files.delete(key);
      },
    );
    e.files.set(key, created);
    sh = created;
  }
  const mine = sh;
  return join(mine, signal, () => {
    if (e.files.get(key) === mine) e.files.delete(key);
  });
}

export function prDiffFileLoader(repoId: string, num: number): DiffFileLoader {
  return (file, { full, signal }) => prDiffFile(repoId, num, file, full, signal);
}

/** Which head the PR's cached diff belongs to ('' = unknown / nothing cached). */
export function prDiffHead(repoId: string, num: number): string {
  return prCache.get(`${repoId}#${num}`)?.head ?? '';
}

/** Tell the cache the PR's current head (from a PR detail load): an entry
 *  for another head is dropped, one that didn't know its head adopts it. */
export function notePrHead(repoId: string, num: number, head: string | null | undefined): void {
  if (head && prCache.has(`${repoId}#${num}`)) prEntry(repoId, num, head);
}
