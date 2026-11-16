// Data loaders and live disk/editor sync helpers for `IdeComparisonModal.svelte`.

import { api, ApiError } from '../../../lib/api/client';
import { gitApi } from '../../../lib/api/git';
import type { Repo } from '../../../lib/api/types';
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
import { detectTextEol, diskEolsOf, normalizeEditorLf, serializeTabDiskContent, sha256HexOfText } from '../store/ideTypes';

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

async function readDiskFile(
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
  } catch (e) {
    // Only a missing file is "absent": a read failure must not become an empty file (a 0-byte
    // Local History label, or a comparison showing the file as deleted).
    if (e instanceof ApiError && e.status === 404) return { content: '', exists: false };
    throw e;
  }
}

/** The live buffer of an open, loaded tab for `absPath` (what "Current" must show, unsaved
 *  edits included), or null when the file is not open. */
export function openTabBuffer(absPath: string): string | null {
  const clean = ide.resolvePath(absPath);
  return ide.tabs.some((t) => t.path === clean && !t.loading && !t.error) ? ide.getBuffer(clean) : null;
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
  return readDiskFile(clean, signal);
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

/**
 * Write the comparison's "Current" side. Returns the text now on disk, or null when
 * nothing was written.
 *
 * - The file is open: the change goes into the buffer as ONE undoable edit (⌘Z still
 *   works), then through the tab's normal save (sha precondition, conflict prompt, line
 *   endings). Unsaved edits in that tab are never replaced silently: ask first.
 * - Not open: a direct write that keeps the line endings of `diskBaseline` (the text the
 *   comparison read) and is refused (409) if the file changed on disk since.
 */
export async function writeCurrentAndSyncIde(
  absPath: string,
  nextContent: string,
  wasMissing = false,
  diskBaseline: string | null = null,
): Promise<string | null> {
  const clean = ide.resolvePath(absPath);
  const tab = ide.tabs.find((t) => t.path === clean);
  if (tab && !tab.loading && !tab.error) {
    if (tab.readOnly || tab.truncated) {
      toasts.error(`${ideBasename(clean)} is read-only`, 'Nothing was changed.');
      return null;
    }
    if (ide.getBuffer(clean) !== tab.savedContent) {
      const ok = await confirmer.ask(
        `“${tab.name}” has unsaved changes. Applying this change saves them to disk together with it.`,
        { title: 'Unsaved changes', confirmLabel: 'Apply and save' },
      );
      if (!ok) return null;
    }
    if (!(await ide.applyAgentEdit(clean, nextContent))) return null;
    if (!(await ide.saveFile(clean))) return null;
    void ideGit.refreshActive();
    return nextContent;
  }
  const root = ideGit.rootContainingPath(clean)?.path || ide.target.root;
  const base = diskBaseline ?? '';
  const disk = serializeTabDiskContent({ eol: detectTextEol(base), diskEols: diskEolsOf(base), savedContent: normalizeEditorLf(base) }, nextContent);
  try {
    const prefs = readIdePreferences();
    // A missing file must still be missing (the empty-content hash is the daemon's "absent").
    const expected = wasMissing ? await sha256HexOfText('') : diskBaseline !== null ? await sha256HexOfText(diskBaseline) : undefined;
    await api.put<{ path: string; size: number; sha256: string }>('/ide/fs/file', {
      target: { ...ide.targetPayload, root },
      path: clean,
      content: disk,
      expected_sha256: expected,
      local_history_enabled: prefs.localHistoryEnabled,
      local_history_retention_days: prefs.localHistoryRetentionDays,
    });
    // A tab still loading (or that failed to) re-reads what was just written.
    if (tab) void ide.openFile(clean, { reload: true, background: true });
    if (wasMissing) await refreshTreeForRestoredPath(root, clean);
    void ideGit.refreshActive();
    return disk;
  } catch (e) {
    if (e instanceof ApiError && e.status === 409) {
      toasts.error(`${ideBasename(clean)} changed on disk`, 'Nothing was written. Reopen the comparison to see the current version.');
    } else {
      toastError(`Couldn’t save ${ideBasename(clean)}`, e);
    }
    return null;
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
  const cur = await readCurrentDiskOrTab(absPath, signal);
  const relPath = ideGit.repoRelativePath(repo, absPath);
  if (!relPath) {
    return { leftText: cur.content, rightText: cur.content, rightExists: cur.exists };
  }
  const revResp = await gitApi.fileAtRev(repo.id, relPath, ref || 'HEAD', signal);
  if (revResp.binary) {
    throw new Error('Binary files cannot be compared as text.');
  }
  if (revResp.truncated) {
    throw new Error('Revision file exceeds the 4 MiB comparison limit.');
  }
  const leftText = revResp.exists ? (revResp.content ?? '') : '';
  return { leftText, rightText: cur.content, rightExists: cur.exists };
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
