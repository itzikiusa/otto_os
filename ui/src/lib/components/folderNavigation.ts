/** Navigation state shared by every folder/file picker instance. */
export interface FolderHistory { paths: string[]; index: number }
export const emptyHistory = (): FolderHistory => ({ paths: [], index: -1 });

/** Record only successfully loaded, canonical paths. A new visit after Back
 *  discards the forward branch, while refreshing the current path preserves it. */
export function recordFolder(history: FolderHistory, path: string): FolderHistory {
  if (history.paths[history.index] === path) return history;
  const paths = [...history.paths.slice(0, history.index + 1), path];
  return { paths, index: paths.length - 1 };
}

export function historyTarget(history: FolderHistory, delta: -1 | 1): number | null {
  const target = history.index + delta;
  return target >= 0 && target < history.paths.length ? target : null;
}

/** A folder finished loading at canonical `path`. A fresh visit (`index`
 *  undefined) is recorded; a Back/Forward step (`index` = the entry walked to)
 *  keeps the branch and only canonicalizes that entry. */
export function settleFolder(history: FolderHistory, path: string, index?: number): FolderHistory {
  if (index === undefined) return recordFolder(history, path);
  const paths = [...history.paths];
  paths[index] = path;
  return { paths, index };
}

/** The folder above `path` (`null` at the root). */
export function parentFolder(path: string): string | null {
  const trimmed = path.replace(/\/+$/, '');
  if (trimmed === '') return null;
  return trimmed.slice(0, trimmed.lastIndexOf('/')) || '/';
}

/** A typed "go to" path, made absolute against the folder being shown:
 *  `/abs`, `~` / `~/x` (the daemon expands it), or relative (`src`, `../x`).
 *  `.`/`..` segments and repeated slashes are folded; `null` for blank input.
 *  The daemon still checks access and canonicalizes (symlinks) on load. */
export function resolveJump(input: string, current: string): string | null {
  const raw = input.trim();
  if (!raw) return null;
  if (raw === '~' || raw.startsWith('~/')) return raw.replace(/\/+$/, '') || '~';
  const joined = raw.startsWith('/') ? raw : `${current.replace(/\/+$/, '')}/${raw}`;
  const out: string[] = [];
  for (const part of joined.split('/')) {
    if (part === '' || part === '.') continue;
    if (part === '..') out.pop();
    else out.push(part);
  }
  return `/${out.join('/')}`;
}

/** The daemon returns absolute POSIX paths. Keep spaces and Unicode intact. */
export function folderCrumbs(path: string): { label: string; path: string }[] {
  const crumbs = [{ label: '/', path: '/' }];
  let parent = '';
  for (const part of path.split('/').filter(Boolean)) {
    parent += `/${part}`;
    crumbs.push({ label: part, path: parent });
  }
  return crumbs;
}

export interface FolderShortcuts { favorites: string[]; recents: string[] }
export const emptyShortcuts = (): FolderShortcuts => ({ favorites: [], recents: [] });

/** Browser preferences never authorize filesystem access; every jump still
 *  goes through the daemon's normal browse permission checks. */
export function parseShortcuts(raw: string | null): FolderShortcuts {
  try {
    const value = JSON.parse(raw ?? '{}');
    const paths = (list: unknown, limit: number) => Array.isArray(list)
      ? [...new Set(list.filter((path): path is string => typeof path === 'string' && path.startsWith('/')))].slice(0, limit)
      : [];
    return { favorites: paths(value?.favorites, Infinity), recents: paths(value?.recents, 20) };
  } catch { return emptyShortcuts(); }
}

export function rememberFolder(shortcuts: FolderShortcuts, path: string): FolderShortcuts {
  return { ...shortcuts, recents: [path, ...shortcuts.recents.filter(entry => entry !== path)].slice(0, 20) };
}

export function toggleFavorite(shortcuts: FolderShortcuts, path: string): FolderShortcuts {
  return { ...shortcuts, favorites: shortcuts.favorites.includes(path)
    ? shortcuts.favorites.filter(entry => entry !== path) : [...shortcuts.favorites, path] };
}
