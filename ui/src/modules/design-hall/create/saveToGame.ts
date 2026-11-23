// "Save to my game": write made things into the workspace folder under a
// sensible name, never over an existing file (a taken name gets -2, -3 …).

import { ApiError } from '../../../lib/api/client';
import { export3dArtifactToWorkspace, exportTtsClipToWorkspace } from '../../../lib/api/media';
import type { ExportWorkspaceResp } from '../../../lib/api/types';

/** Try `path(1)`, `path(2)`… until a name is free (409 = taken). */
export async function saveFree(
  write: (relativePath: string) => Promise<ExportWorkspaceResp>,
  path: (n: number) => string,
): Promise<ExportWorkspaceResp> {
  for (let n = 1; n < 50; n++) {
    try {
      return await write(path(n));
    } catch (e) {
      if (!(e instanceof ApiError && e.status === 409)) throw e;
    }
  }
  throw new Error('Too many files with that name');
}

export const saveModel = (artifactId: string, path: (n: number) => string, versionId?: string | null) =>
  saveFree((relative_path) => export3dArtifactToWorkspace(artifactId, { relative_path, version_id: versionId ?? null }), path);

export const saveClip = (clipId: string, path: (n: number) => string) =>
  saveFree((relative_path) => exportTtsClipToWorkspace(clipId, { relative_path }), path);

/** Why a save failed, in plain words. */
export function saveProblem(e: unknown): string {
  const m = e instanceof Error ? e.message : '';
  if (/root_path|root path|no root/i.test(m)) return 'This workspace has no game folder yet. Give it a folder in Settings, then try again.';
  return 'Please try again in a moment.';
}
