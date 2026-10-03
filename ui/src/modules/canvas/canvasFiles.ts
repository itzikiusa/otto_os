// Excalidraw images out of the autosave body (perf R2).
//
// The daemon stores a scene's pasted images ONCE in a content-addressed file
// store and keeps `otto-canvas-file:<sha256>` refs in the document. The editor
// reads the doc with refs (`GET /canvas/scenes/{id}?files=ref`), resolves each
// ref once through the immutable `GET /canvas/files/{sha}` (the webview's HTTP
// cache serves a reopened board), and autosaves refs back — so a board with a
// 3 MB screenshot autosaves a few KB instead of re-sending (and main-thread
// stringifying) the base64 every 700 ms. A file the server hasn't seen yet
// goes inline exactly once; its sha (sha256 of the data-URL text, the same
// digest the server uses) is learned after that save lands.
//
// The pure helpers here are unit-tested (ui/unit/canvasFiles.test.ts, via
// canvasFileRefs.ts + canvasFileCache.ts); the fetch/digest ones are thin wrappers.

import { baseUrl, getToken } from '../../lib/api/client';
import { FileCache } from './canvasFileCache';
import { refSha, type ExFile } from './canvasFileRefs';

export { FILE_REF_PREFIX, refSha, filesForSave, sha256Hex } from './canvasFileRefs';

// One fetch per sha while a board uses it, bounded to 64 MB (LRU) and released
// when the last board using it unmounts (the HTTP cache covers a reopen).
const cache = new FileCache(async (sha) => {
  const token = getToken();
  const headers: Record<string, string> = token ? { Authorization: `Bearer ${token}` } : {};
  const resp = await fetch(`${baseUrl()}/api/v1/canvas/files/${sha}`, { headers });
  if (!resp.ok) throw new Error(`canvas file ${sha.slice(0, 8)}: ${resp.status}`);
  return resp.text();
});

/** A file's data URL by sha for `owner` (authed; cached; a failure is not cached). */
export function fetchFile(sha: string, owner: unknown): Promise<string> {
  return cache.get(sha, owner);
}

/** A board unmounted: free the files no other open board uses. */
export function releaseFiles(owner: unknown): void {
  cache.release(owner);
}

/**
 * Resolve a scene's `files` map for Excalidraw: ref entries get their data URL
 * fetched (and `known` learns id → sha); inline entries pass through. Files
 * whose fetch fails are left out (Excalidraw shows its placeholder) rather
 * than failing the whole load. `owner` = the board (released on unmount via
 * `releaseFiles`); `skip` = ids the editor already holds.
 */
export async function resolveFiles(
  files: Record<string, ExFile> | null | undefined,
  known: Map<string, string>,
  owner: unknown,
  skip: ReadonlySet<string> = new Set(),
): Promise<ExFile[]> {
  const entries = Object.entries(files ?? {}).filter(([id, f]) => {
    if (!f || typeof f !== 'object') return false;
    const sha = refSha(f.dataURL);
    if (sha) known.set(id, sha); // held server-side, even when we skip the fetch
    return !skip.has(id);
  });
  const out = await Promise.all(
    entries.map(async ([, f]) => {
      const sha = refSha(f.dataURL);
      if (!sha) return f;
      try {
        return { ...f, dataURL: await fetchFile(sha, owner) };
      } catch {
        return null;
      }
    }),
  );
  return out.filter((f): f is ExFile => f !== null);
}
