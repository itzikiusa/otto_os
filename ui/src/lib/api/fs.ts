// Session-scoped working-folder files (PC-1, access-control-4-03).
//
// `GET /sessions/{id}/files/{browse,read}?path=<rel>` serve a session's cwd to
// its owner (or a ws Admin / root) at ws Viewer — paths RELATIVE to the cwd,
// 400 for an absolute path, `..` or an escaping symlink. They exist because
// Otto's engines put many session cwds inside its data dir (workflow
// worktrees, persona folders, goal-loop / scheduled-task / swarm work dirs),
// which `/fs/*` refuses to every non-root caller.
//
// `sessionFs` adapts them to the Side panel's FileTree, which speaks ABSOLUTE
// paths (breadcrumbs, history, go-to): `/fs/*` stays the first choice, and a
// 403 for a path inside the session's cwd retries through the session route,
// whose relative answer is mapped back onto the cwd.
import { api, ApiError } from './client';
import type { FsBrowse, FsRead } from './types';

const enc = encodeURIComponent;

/** List one directory of a session's cwd (`''` = its root). */
export function browseSessionFiles(sessionId: string, path: string, signal?: AbortSignal): Promise<FsBrowse> {
  return api.get<FsBrowse>(`/sessions/${enc(sessionId)}/files/browse?path=${enc(path)}`, signal);
}

/** Read one file of a session's cwd (same `FsRead` shape and 400 KiB bound as `/fs/read`). */
export function readSessionFile(sessionId: string, path: string, signal?: AbortSignal): Promise<FsRead> {
  return api.get<FsRead>(`/sessions/${enc(sessionId)}/files/read?path=${enc(path)}`, signal);
}

/** `path` relative to `cwd` (`''` for the cwd itself), or null when it lies
 *  outside. A path that is already relative is returned as is. Pure. */
export function relativeToCwd(cwd: string, path: string): string | null {
  const root = cwd.replace(/\/+$/, '');
  if (!path.startsWith('/')) return path;
  if (path === root || path === `${root}/`) return '';
  if (root && path.startsWith(`${root}/`)) return path.slice(root.length + 1);
  return null;
}

/** `rel` (a session-route answer) back onto the cwd as an absolute path. Pure. */
export function onCwd(cwd: string, rel: string): string {
  const root = cwd.replace(/\/+$/, '');
  return rel ? `${root}/${rel}` : root;
}

/** Map a session-route listing onto absolute paths. At the cwd itself the
 *  parent is null: `/fs/*` already refused this tree, so "Up" would too. Pure. */
export function absoluteBrowse(cwd: string, b: FsBrowse): FsBrowse {
  return {
    ...b,
    path: onCwd(cwd, b.path),
    parent: b.parent === null ? null : onCwd(cwd, b.parent),
    entries: b.entries.map((e) => ({ ...e, path: onCwd(cwd, e.path) })),
  };
}

const isForbidden = (e: unknown): boolean => e instanceof ApiError && e.status === 403;

/** FileTree `lister` / `reader` for an agent session's Files tab. */
export function sessionFs(sessionId: string, cwd: string) {
  return {
    lister: async (path: string, signal?: AbortSignal): Promise<FsBrowse> => {
      try {
        return await api.get<FsBrowse>(`/fs/browse?path=${enc(path)}&files=true`, signal);
      } catch (e) {
        const rel = relativeToCwd(cwd, path);
        if (!isForbidden(e) || rel === null) throw e;
        return absoluteBrowse(cwd, await browseSessionFiles(sessionId, rel, signal));
      }
    },
    reader: async (path: string, signal?: AbortSignal): Promise<FsRead> => {
      try {
        return await api.get<FsRead>(`/fs/read?path=${enc(path)}`, signal);
      } catch (e) {
        const rel = relativeToCwd(cwd, path);
        if (!isForbidden(e) || rel === null || rel === '') throw e;
        const r = await readSessionFile(sessionId, rel, signal);
        return { ...r, path: onCwd(cwd, r.path) };
      }
    },
  };
}
