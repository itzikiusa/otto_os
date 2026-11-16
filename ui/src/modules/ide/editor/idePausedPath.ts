// Whether a debugger's paused file (absolute, relative, `file://`, or under
// `/private`) names the open tab's file, given the project's roots.

import { fileUriToPath, normalizeMacPrivatePath } from '../../../lib/fileUri';

function canonDebugPath(raw: string): string {
  const clean = fileUriToPath(raw).replace(/\\/g, '/').replace(/^\.\//, '').replace(/\/+$/, '');
  return normalizeMacPrivatePath(clean);
}

export function matchesPausedDebugPath(pausedFile: string | null | undefined, tabPath: string, projectRoots: string[]): boolean {
  if (!pausedFile || !tabPath) return false;
  if (pausedFile === tabPath) return true;
  const p = canonDebugPath(pausedFile);
  const t = canonDebugPath(tabPath);
  if (!p || !t) return false;
  if (p === t) return true;
  const roots = projectRoots.filter(Boolean).map(canonDebugPath);
  if (!p.startsWith('/')) {
    for (const r of roots) {
      if (`${r}/${p}` === t) return true;
    }
    return t.endsWith(`/${p}`);
  }
  if (!t.startsWith('/')) {
    for (const r of roots) {
      if (`${r}/${t}` === p) return true;
    }
    return p.endsWith(`/${t}`);
  }
  return false;
}
