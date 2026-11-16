// Lazy-loaded tab context menus, code-dive (⌘B / ⌘⌥B) resolution, and Git blame
// fetching for `IdeTabBar` and `ideEditorStoreBridge`.

import { api } from '../../../lib/api/client';
import { gitApi } from '../../../lib/api/git';
import type { BlameLine, IdeGitBaseResp } from '../../../lib/api/types';
import { copyWithToast } from '../../../lib/clipboard';
import { ctxMenu, type MenuItem } from '../../../lib/contextmenu.svelte';
import { isBroadNonProjectRootClient, normalizeMacPrivatePath } from '../../../lib/fileUri';
import { ideGit } from '../git/ideGitController.svelte';
import { ide, ideBasename, type IdeOpenTab } from '../ideStore.svelte';
import { ideLayout } from '../shell/ideLayout.svelte';
import { ideAgent } from '../store/ideAgentController.svelte';
import { ideEditorChrome } from './ideEditorChrome.svelte';
import { normalizeEditorLf, type IdeReferenceLocation } from '../store/ideTypes';
import {
  closeAllIdeTabs,
  closeUnmodifiedIdeTabs,
  flushAutoLocalHistorySnapshot,
  formatIdeTabDisplayName,
  getIdeProjectFolders,
  idePreviewMode,
  isIdeTabPinned,
  openIdeTerminalForPath,
  pushIdeNavLocation,
  putIdeLocalHistoryLabel,
  revealInIdeProjectTree,
  toggleIdeTabPinned,
} from './ideEditorStoreBridge.svelte';
import { ideParentFolderHint } from './ideCrumbs';
import { resolveLocalFastDive } from './ideLangAnalyzer';

export function openIdeTabContextMenu(
  anchor: HTMLElement | MouseEvent,
  tab: IdeOpenTab,
  onsave: (path: string) => void,
): void {
  const tabDirty = tab.dirty ?? (tab.content !== tab.savedContent);
  const pinned = isIdeTabPinned(tab);
  const items: MenuItem[] = [
    {
      label: 'Save',
      icon: 'download',
      hint: '⌘S',
      disabled: tab.readOnly || !tabDirty,
      action: () => onsave(tab.path),
    },
    { label: pinned ? 'Unpin tab' : 'Pin tab', icon: 'pin', action: () => toggleIdeTabPinned(tab.path) },
    { separator: true },
    { label: 'Split right', icon: 'split', action: () => ide.openInSplit(tab.path, 'right') },
    { label: 'Split down', icon: 'unified', action: () => ide.openInSplit(tab.path, 'down') },
    { separator: true },
    { label: 'Close', icon: 'x', disabled: pinned, action: () => void ide.closeTab(tab.path) },
    { label: 'Close others', disabled: ide.tabs.length <= 1, action: () => ide.closeOtherTabs(tab.path) },
    { label: 'Close unmodified', action: () => void closeUnmodifiedIdeTabs() },
    { label: 'Close all', action: () => void closeAllIdeTabs() },
    { separator: true },
    {
      label: 'Local History: Show History…',
      icon: 'clock',
      action: () => {
        flushAutoLocalHistorySnapshot(tab.path);
        window.dispatchEvent(new CustomEvent('otto:ide-open-local-history', { detail: { path: tab.path } }));
      },
    },
    { label: 'Local History: Put Label…', icon: 'tag', action: () => void putIdeLocalHistoryLabel(tab.path, tab.content) },
    { separator: true },
    { label: 'Copy path', icon: 'copy', action: () => void copyWithToast(tab.path, 'file path') },
    { label: 'Copy relative path', icon: 'copy', action: () => void copyWithToast(ide.relativePath(tab.path), 'relative path') },
    { label: 'Reveal in project tree', icon: 'folder', action: () => void revealInIdeProjectTree(tab.path) },
    { label: 'Open in terminal', icon: 'terminal', action: () => void openIdeTerminalForPath(tab.path) },
    { label: 'Add to Agent context', icon: 'assistant', action: () => void ideAgent.attachFile(tab.path, { openPanel: true }) },
  ];
  if (anchor instanceof HTMLElement) ctxMenu.showAt(anchor, items, { align: 'start' });
  else ctxMenu.show(anchor, items);
}

export function openIdeOverflowTabsMenu(anchor: HTMLElement | null): void {
  const items: MenuItem[] = ide.tabs.map((t) => {
    const dirty = t.dirty ?? (t.content !== t.savedContent);
    return {
      label: `${formatIdeTabDisplayName(t, ide.tabs, 34)}${dirty ? ' •' : ''}`,
      icon: t.path === ide.activePath ? 'check' : isIdeTabPinned(t) ? 'pin' : t.readOnly ? 'lock' : 'file',
      // The parent folder, not the whole relative path (which squeezed the name to a stub).
      hint: ideParentFolderHint(ide.relativePath(t.path)),
      action: () => {
        if (ide.activePath && ide.activePath !== t.path) flushAutoLocalHistorySnapshot(ide.activePath);
        ide.activePath = t.path;
      },
    };
  });
  // The tab strip's Editor | Preview control, again here only on a phone (a menu row is
  // easier to hit there); on a desktop it already sits next to this menu's button.
  const pvModes = ideLayout.stack ? idePreviewMode.available(ide.activePath) : [];
  const pvCur = idePreviewMode.modeFor(ide.activePath);
  const pvLabel = { source: 'Editor', split: 'Editor and preview', preview: 'Preview' } as const;
  const pvIcon = { source: 'editorLines', split: 'editorSplit', preview: 'image' } as const;
  if (pvModes.length) {
    items.push({ separator: true }, ...pvModes.map((m) => ({ label: pvLabel[m], icon: pvIcon[m], checked: pvCur === m, action: () => idePreviewMode.set(ide.activePath, m) })));
  }
  items.push(
    { separator: true },
    // Stacked layout: the editor toolbar row is hidden to give the code that row.
    ...(ideLayout.stack
      ? [{ label: 'Show editor toolbar', icon: 'panel' as const, checked: ideEditorChrome.subbarShown, action: () => (ideEditorChrome.subbarShown = !ideEditorChrome.subbarShown) }]
      : []),
    { label: 'Close unmodified tabs', action: () => void closeUnmodifiedIdeTabs() },
    { label: 'Close all tabs', icon: 'x', action: () => void closeAllIdeTabs() },
  );
  ctxMenu.showAt(anchor, items, { align: 'end' });
}

function isImportBindingLine(lineText: string): boolean {
  return /^\s*(?:import\b|(?:const|let|var)\s+.*=\s*require\()/.test(lineText);
}

export async function runPerformIdeDive(
  mode: 'definition' | 'implementation',
  fromPath: string,
  target: { path?: string; line: number; column: number; symbol?: string; originLine?: number; originColumn?: number },
  content: string,
): Promise<void> {
  const originLine = target.originLine ?? (ide.cursorInfo.line || target.line);
  const originCol = target.originColumn ?? (ide.cursorInfo.col || target.column);
  pushIdeNavLocation(fromPath, originLine, originCol);

  let sameFileImportFallback: { path: string; line: number; column: number } | null = null;
  if (target.path) {
    const sameFile = normalizeMacPrivatePath(ide.resolvePath(target.path)) === normalizeMacPrivatePath(ide.resolvePath(fromPath));
    const destLineText = sameFile ? (content.split(/\r?\n/)[target.line - 1] ?? '') : '';
    if (mode === 'definition' && sameFile && isImportBindingLine(destLineText)) {
      sameFileImportFallback = { path: target.path, line: target.line, column: target.column };
    } else {
      await ide.openFile(target.path, { line: target.line, col: target.column, preview: false });
      pushIdeNavLocation(target.path, target.line, target.column);
      return;
    }
  }

  if (mode === 'definition') {
    const localHit = resolveLocalFastDive(fromPath, ide.target.root, content, originLine, target.symbol);
    if (localHit) {
      await ide.openFile(localHit.path, { line: localHit.line, col: localHit.column, preview: false });
      pushIdeNavLocation(localHit.path, localHit.line, localHit.column);
      return;
    }
  }

  const endpoint = mode === 'implementation' ? '/ide/lsp/implementation' : '/ide/lsp/definition';
  try {
    const resp = await api.post<{ symbol: string; locations: IdeReferenceLocation[] }>(endpoint, {
      target: { ...ide.targetPayload, additional_roots: getIdeProjectFolders().slice(1) },
      path: fromPath, line: originLine, column: originCol, symbol: target.symbol, content,
    });
    const locs = resp.locations ?? [];
    if (locs.length === 1) {
      const hit = locs[0];
      await ide.openFile(hit.path, { line: hit.line, col: hit.column, preview: false });
      pushIdeNavLocation(hit.path, hit.line, hit.column);
      return;
    }
    if (locs.length > 1) {
      const items = locs.slice(0, 40).map((loc) => ({
        label: `${ideBasename(loc.path)}:${loc.line}`,
        hint: loc.preview || ide.relativePath(loc.path),
        icon: 'file' as const,
        action: () => {
          void ide.openFile(loc.path, { line: loc.line, col: loc.column, preview: false });
          pushIdeNavLocation(loc.path, loc.line, loc.column);
        },
      }));
      const anchor = document.querySelector<HTMLElement>('[data-testid="ide-editor-area"]');
      if (anchor) ctxMenu.showAt(anchor, items, { align: 'start' });
      else await ide.openFile(locs[0].path, { line: locs[0].line, col: locs[0].column, preview: false });
      return;
    }
  } catch { /* fall back below */ }

  if (sameFileImportFallback) {
    await ide.openFile(sameFileImportFallback.path, { line: sameFileImportFallback.line, col: sameFileImportFallback.column, preview: false });
    pushIdeNavLocation(sameFileImportFallback.path, sameFileImportFallback.line, sameFileImportFallback.column);
    return;
  }
  await ide.findReferences(fromPath, originLine, originCol, target.symbol);
}

type RawBlameEntry = Partial<BlameLine> & {
  start_line?: number;
  end_line?: number;
  line?: number;
  commit?: string;
};

export async function runFetchIdeGitBlame(_rootPath: string, filePath: string, signal?: AbortSignal): Promise<Record<number, string> | null> {
  try {
    const normFile = normalizeMacPrivatePath(filePath);
    let repoId = '';
    let relPath = '';
    const resolvedRepo = await ideGit.resolveRepoForPath(normFile);
    if (resolvedRepo && !isBroadNonProjectRootClient(resolvedRepo.path)) {
      const rel = ideGit.repoRelativePath(resolvedRepo, normFile);
      if (rel) {
        repoId = resolvedRepo.id;
        relPath = rel;
      }
    }
    if (!repoId || !relPath) {
      const repos = await api.bg.get<Array<{ id: string; path: string }>>('/git/repos', signal);
      const matches = (repos ?? [])
        .filter((r) => !isBroadNonProjectRootClient(r.path))
        .map((r) => ({ id: r.id, root: normalizeMacPrivatePath(r.path).replace(/\/+$/, '') }))
        .filter((r) => normFile === r.root || normFile.startsWith(`${r.root}/`))
        .sort((a, b) => b.root.length - a.root.length);
      if (!matches.length) return null;
      repoId = matches[0].id;
      relPath = normFile.slice(matches[0].root.length).replace(/^\/+/, '');
    }
    if (!relPath) return null;
    const resp = await gitApi.blame(repoId, relPath, undefined, {
      signal,
      background: true,
    });
    const list: RawBlameEntry[] = Array.isArray(resp) ? resp : (resp.lines ?? []);
    if (!list.length) return null;
    const map: Record<number, string> = {};
    for (const item of list) {
      const sha = item.short_sha || (item.sha ?? item.commit ?? '').slice(0, 7);
      const label = [item.author, sha, item.summary].filter(Boolean).join(' · ').slice(0, 56);
      const start = item.line_start ?? item.start_line ?? item.line;
      if (!label || start == null) continue;
      const count = Math.max(
        1,
        item.count ?? (item.end_line != null ? item.end_line - start + 1 : 1),
      );
      for (let ln = start; ln < start + count; ln++) map[ln] = label;
    }
    return Object.keys(map).length > 0 ? map : null;
  } catch {
    return null;
  }
}

/**
 * Resolve the Git `HEAD` baseline content for `filePath` via `/ide/fs/git-base`
 * so editor gutter change bars (`add` green / `mod` light-blue / `del` triangle)
 * compare the live buffer directly against Git `HEAD` — independent of repo
 * registration races, staged-vs-unstaged state, or session restarts.
 *
 * Transient network/API errors are intentionally NOT swallowed here so callers
 * do not memoize a stale `diskSavedContent` fallback as if `HEAD` matched disk.
 */
export async function runResolveIdeGitBaseContent(
  filePath: string,
  diskSavedContent: string | null,
  signal?: AbortSignal,
): Promise<string | null | undefined> {
  if (
    !filePath ||
    diskSavedContent == null ||
    filePath.startsWith('untitled:') ||
    filePath.startsWith('jdt://') ||
    filePath.startsWith('jar:')
  ) {
    return undefined;
  }
  const normFile = ide.target.kind === 'local' ? normalizeMacPrivatePath(filePath) : filePath;
  const folders = getIdeProjectFolders()
    .map((f) => (ide.target.kind === 'local' ? normalizeMacPrivatePath(f) : f).replace(/\/+$/, ''))
    .filter((f) => f && (normFile === f || normFile.startsWith(`${f}/`)))
    .sort((a, b) => b.length - a.length);
  const root = folders[0] || ide.target.root;
  const resp = await api.bg.get<IdeGitBaseResp>(
    `/ide/fs/git-base?${ide.targetQuery({ root, path: filePath })}`,
    signal,
  );
  if (!resp.in_repo) return undefined;
  if (resp.binary || resp.truncated) return null;
  if (!resp.head_exists) return '';
  return normalizeEditorLf(resp.content ?? '');
}
