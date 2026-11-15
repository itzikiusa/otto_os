// Data loaders and live disk/editor sync helpers for `IdeComparisonModal.svelte`.

import { api } from '../../../lib/api/client';
import type { DiffResp, Repo } from '../../../lib/api/types';
import type {
  IdeHistoryEntryResp,
  IdeHistoryListResp,
  IdeHistoryRestoreResp,
  IdeLocalHistoryEntry,
} from '../../../lib/api/types/ide-intelligence';
import { getLatestClipboardForCompare } from '../../../lib/components/cm-ide-history';
import { readIdePreferences } from '../../../lib/components/cm-ide-prefs';
import { confirmer } from '../../../lib/confirm.svelte';
import { toasts } from '../../../lib/toast.svelte';
import { toastError } from '../../../lib/toastError';
import { ideGit } from '../git/ideGitController.svelte';
import { ide, ideBasename, type IdeFileReadResp } from '../ideStore.svelte';
import { reconstructFileFromDiff } from './ideCompareEngine';

export type IdeCompareMode =
  | 'local-history'
  | 'git-revision'
  | 'git-branch'
  | 'clipboard'
  | 'file';

export type {
  IdeLocalHistoryEntry,
  IdeHistoryListResp as HistoryListResp,
  IdeHistoryEntryResp as HistoryEntryResp,
  IdeHistoryRestoreResp as HistoryRestoreResp,
};

export async function readDiskFileOnly(
  absPath: string,
  signal?: AbortSignal,
): Promise<{ content: string; exists: boolean }> {
  const clean = ide.resolvePath(absPath);
  try {
    const root = ideGit.rootContainingPath(clean)?.path || ide.target.root;
    const res = await api.get<IdeFileReadResp>(
      `/ide/fs/file?${ide.targetQuery({ root, path: clean })}`,
      signal,
    );
    return { content: res.content, exists: true };
  } catch {
    return { content: '', exists: false };
  }
}

export async function readCurrentDiskOrTab(
  absPath: string,
  signal?: AbortSignal,
): Promise<{ content: string; exists: boolean }> {
  const clean = ide.resolvePath(absPath);
  const openTab = ide.tabs.find((t) => t.path === clean && !t.loading && !t.error);
  if (openTab) {
    return { content: ide.getBuffer(clean), exists: true };
  }
  return readDiskFileOnly(clean, signal);
}

export async function refreshTreeForRestoredPath(
  fallbackRoot: string,
  restoredPath: string,
  relPath?: string | null,
): Promise<string> {
  await ide.loadTree();
  const sampleEntry = ide.treeRootEntries[0]?.path;
  const canonicalRoot = sampleEntry
    ? sampleEntry.slice(0, sampleEntry.lastIndexOf('/'))
    : fallbackRoot.replace(/\/+$/, '');
  const cleanRoot = fallbackRoot.replace(/\/+$/, '');
  const derivedRel =
    relPath?.replace(/^\/+/, '') ||
    (restoredPath.startsWith(`${cleanRoot}/`) ? restoredPath.slice(cleanRoot.length + 1) : '');
  const resolved = derivedRel ? `${canonicalRoot}/${derivedRel}` : restoredPath;
  const slash = resolved.lastIndexOf('/');
  const parentDir = slash > canonicalRoot.length ? resolved.slice(0, slash) : '';
  if (parentDir) await ide.loadTree(parentDir);
  await ide.revealInProjectTree(resolved);
  return resolved;
}

export async function writeCurrentAndSyncIde(
  absPath: string,
  nextContent: string,
  wasMissing = false,
): Promise<boolean> {
  const clean = ide.resolvePath(absPath);
  const root = ideGit.rootContainingPath(clean)?.path || ide.target.root;
  try {
    const prefs = readIdePreferences();
    const resp = await api.put<{ path: string; size: number; sha256: string }>('/ide/fs/file', {
      target: { ...ide.targetPayload, root },
      path: clean,
      content: nextContent,
      local_history_enabled: prefs.localHistoryEnabled,
      local_history_retention_days: prefs.localHistoryRetentionDays,
    });
    const tab = ide.tabs.find((t) => t.path === clean);
    if (tab) {
      if (ide.activePath === clean) {
        await ide.applyAgentEdit(clean, nextContent);
      } else {
        ide.updateBuffer(clean, nextContent);
      }
      tab.content = nextContent;
      tab.savedContent = nextContent;
      tab.dirty = false;
      tab.sha256 = resp.sha256;
    }
    if (wasMissing) await refreshTreeForRestoredPath(root, clean);
    void ideGit.refreshActive();
    return true;
  } catch (e) {
    toastError(`Couldn’t save ${ideBasename(clean)}`, e);
    return false;
  }
}

export async function readClipboardForCompare(override?: string): Promise<string> {
  if (typeof override === 'string') return override;
  return getLatestClipboardForCompare();
}

export async function fetchGitRefFileContent(
  repo: Repo,
  absPath: string,
  ref: string,
  signal?: AbortSignal,
): Promise<{ leftText: string; rightText: string; rightExists: boolean }> {
  const [cur, disk] = await Promise.all([
    readCurrentDiskOrTab(absPath, signal),
    readDiskFileOnly(absPath, signal),
  ]);
  const relPath = ideGit.repoRelativePath(repo, absPath);
  if (!relPath) {
    return { leftText: disk.content, rightText: cur.content, rightExists: cur.exists };
  }
  const encRepo = encodeURIComponent(repo.id);
  const encPath = encodeURIComponent(relPath);
  let headText = disk.content;
  try {
    const workingDiff = await api.get<DiffResp>(
      `/repos/${encRepo}/diff?target=working&path=${encPath}&full=true`,
      signal,
    );
    headText = reconstructFileFromDiff(disk.content, workingDiff, 'reverse');
  } catch {
    /* working tree already matches HEAD */
  }
  if (!ref || ref === 'HEAD') {
    return { leftText: headText, rightText: cur.content, rightExists: cur.exists };
  }
  const rangeDiff = await api.get<DiffResp>(
    `/repos/${encRepo}/diff?target=${encodeURIComponent(`range:HEAD..${ref}`)}&path=${encPath}&full=true`,
    signal,
  );
  const refText = reconstructFileFromDiff(headText, rangeDiff, 'forward');
  return { leftText: refText, rightText: cur.content, rightExists: cur.exists };
}

export async function promptPutLocalHistoryLabel(
  root: string,
  targetPath: string,
  opts: { content?: string; isDir?: boolean } = {},
): Promise<boolean> {
  const cleanTarget = ide.resolvePath(targetPath || root);
  const label = await confirmer.promptText(
    `Named checkpoint label for “${ideBasename(cleanTarget)}” in Local History`,
    {
      title: 'Put Local History label',
      confirmLabel: 'Save label',
      placeholder: 'Before refactoring',
    },
  );
  if (!label || !label.trim()) return false;
  try {
    const content = opts.isDir
      ? ''
      : typeof opts.content === 'string'
        ? opts.content
        : (await readCurrentDiskOrTab(cleanTarget)).content;
    const folders = ide.targetPayload.project_folders ?? [];
    const effectiveRoot =
      root || folders.find((f) => cleanTarget === f || cleanTarget.startsWith(`${f}/`)) || ide.target.root;
    await api.post('/ide/history/snapshot', {
      target: {
        ...ide.targetPayload,
        root: effectiveRoot,
        additional_roots: folders.filter((f) => f !== effectiveRoot),
      },
      root: effectiveRoot,
      path: cleanTarget,
      label: label.trim(),
      content,
    });
    toasts.success('Saved Local History label', label.trim());
    return true;
  } catch (e) {
    toastError('Couldn’t save Local History label', e);
    return false;
  }
}
