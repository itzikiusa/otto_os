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
