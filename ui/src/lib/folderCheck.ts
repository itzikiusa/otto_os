// Pre-flight for a typed working folder (S20-04 / S14-11), shared by New
// Session, the Agents first-run coach and onboarding's workspace step: ask
// the daemon whether the folder exists BEFORE starting an agent there. The
// daemon's spawn creates a missing cwd, so a typo used to start the agent in
// a fresh, empty stray folder.
//
// It asks `/fs/stat` (S14-306): one stat, no listing. `/fs/browse` read every
// entry and probed each child's `.git` — seconds on `~` or a monorepo root —
// and held one of the four folder-picker permits meanwhile. A 404 from stat
// is confirmed with `/fs/browse` (a daemon that predates `/fs/stat` answers
// an unknown route with 404 too; for a truly missing folder browse fails fast).
//
// Only a definite answer blocks — not a folder, or 400/404 (no such folder).
// A busy/slow/forbidden check (409 / 502 / 403) or a network blip doesn't:
// the create itself still reports a real failure.

import type { FsStat } from './api/types';

export type FolderCheck = { ok: true; path: string } | { ok: false; message: string };

/** `get` = `(url) => api.get(url)`. */
export async function checkFolder(path: string, get: (url: string) => Promise<unknown>): Promise<FolderCheck> {
  const p = path.trim();
  if (p === '') return { ok: false, message: 'Choose a folder first.' };
  try {
    const st = (await get(statPath(p))) as FsStat;
    if (!st.is_dir) return blocked('it is a file, not a folder');
    return { ok: true, path: st.path };
  } catch (e) {
    if (statusOf(e) !== 404) return verdict(e, p);
    try {
      const view = (await get(browsePath(p))) as { path: string };
      return { ok: true, path: view.path };
    } catch (e2) {
      return verdict(e2, p);
    }
  }
}

function statusOf(e: unknown): unknown {
  return (e as { status?: unknown } | null)?.status;
}

function blocked(why: string): FolderCheck {
  return { ok: false, message: `Otto can’t open that folder (${why}). Check the path, or choose it with Browse.` };
}

function verdict(e: unknown, p: string): FolderCheck {
  const status = statusOf(e);
  if (status === 400 || status === 404) return blocked(e instanceof Error && e.message ? e.message : 'it doesn’t exist');
  return { ok: true, path: p };
}

export function statPath(path: string): string {
  return `/fs/stat?path=${encodeURIComponent(path)}`;
}

export function browsePath(path: string): string {
  return `/fs/browse?path=${encodeURIComponent(path)}`;
}
