// Pre-flight for a typed working folder (S20-04 / S14-11), shared by New
// Session, the Agents first-run coach and onboarding's workspace step: ask
// the daemon's `/fs/browse` whether the folder exists BEFORE starting an agent
// there. The daemon's spawn creates a missing cwd, so a typo used to start the
// agent in a fresh, empty stray folder.
//
// Only a definite answer blocks — 400 (not a directory) / 404 (no such
// folder). A busy/slow/forbidden browse (409 / 502 / 403) or a network blip
// doesn't: the create itself still reports a real failure.

export type FolderCheck = { ok: true; path: string } | { ok: false; message: string };

/** `browse` = `(path) => api.get<FsBrowse>('/fs/browse?path=…')`. */
export async function checkFolder(path: string, browse: (path: string) => Promise<{ path: string }>): Promise<FolderCheck> {
  const p = path.trim();
  if (p === '') return { ok: false, message: 'Choose a folder first.' };
  try {
    const view = await browse(p);
    return { ok: true, path: view.path };
  } catch (e) {
    const status = (e as { status?: unknown } | null)?.status;
    if (status === 400 || status === 404) {
      const why = e instanceof Error && e.message ? e.message : 'it doesn’t exist';
      return { ok: false, message: `Otto can’t open that folder (${why}). Check the path, or choose it with Browse.` };
    }
    return { ok: true, path: p };
  }
}

export function browsePath(path: string): string {
  return `/fs/browse?path=${encodeURIComponent(path)}`;
}
