import type { RepoStatusResp } from '../../../lib/api/types';

/** Per-file WIP diff URL. A staged rename sends its origin too, or the new
 *  name alone reads as a whole-file add (+N −0). */
export function wipDiffUrl(repoId: string, target: string, path: string, changes: RepoStatusResp['changes']): string {
  const orig = target === 'staged' ? changes.find((c) => c.path === path)?.orig_path : null;
  const old = orig ? `&old_path=${encodeURIComponent(orig)}` : '';
  return `/repos/${repoId}/diff?target=${target}&path=${encodeURIComponent(path)}${old}`;
}
