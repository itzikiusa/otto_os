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
// The pure helpers here are unit-tested (ui/unit/canvasFiles.test.ts, via canvasFileRefs.ts); the
// fetch/digest ones are thin wrappers.

import { baseUrl, getToken } from '../../lib/api/client';
import { refSha, type ExFile } from './canvasFileRefs';

export { FILE_REF_PREFIX, refSha, filesForSave, sha256Hex } from './canvasFileRefs';

// One fetch per sha per page load (the HTTP cache covers reloads).
const fetched = new Map<string, Promise<string>>();

/** A file's data URL by sha (authed; cached; a failure is not cached). */
export function fetchFile(sha: string): Promise<string> {
  const hit = fetched.get(sha);
  if (hit) return hit;
  const p = (async () => {
    const token = getToken();
    const headers: Record<string, string> = token ? { Authorization: `Bearer ${token}` } : {};
    const resp = await fetch(`${baseUrl()}/api/v1/canvas/files/${sha}`, { headers });
    if (!resp.ok) throw new Error(`canvas file ${sha.slice(0, 8)}: ${resp.status}`);
    return resp.text();
  })();
  fetched.set(sha, p);
  p.catch(() => fetched.delete(sha));
  return p;
}

/**
 * Resolve a scene's `files` map for Excalidraw: ref entries get their data URL
 * fetched (and `known` learns id → sha); inline entries pass through. Files
 * whose fetch fails are left out (Excalidraw shows its placeholder) rather
 * than failing the whole load. `skip` = ids the editor already holds.
 */
export async function resolveFiles(
  files: Record<string, ExFile> | null | undefined,
  known: Map<string, string>,
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
        return { ...f, dataURL: await fetchFile(sha) };
      } catch {
        return null;
      }
    }),
  );
  return out.filter((f): f is ExFile => f !== null);
}
