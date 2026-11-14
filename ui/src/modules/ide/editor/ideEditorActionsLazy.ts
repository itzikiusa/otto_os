// Lazy-loaded tab context menus, code-dive (⌘B / ⌘⌥B) resolution, and Git blame
// fetching for `IdeTabBar` and `ideEditorStoreBridge`.

import { api } from '../../../lib/api/client';
import { copyWithToast } from '../../../lib/clipboard';
import { ctxMenu, type MenuItem } from '../../../lib/contextmenu.svelte';
import { ide, ideBasename, type IdeOpenTab } from '../ideStore.svelte';
import { ideAgent } from '../store/ideAgentController.svelte';
import type { IdeReferenceLocation } from '../store/ideTypes';
import {
  closeAllIdeTabs,
  closeUnmodifiedIdeTabs,
  flushAutoLocalHistorySnapshot,
  formatIdeTabDisplayName,
  getIdeProjectFolders,
  isIdeTabPinned,
  openIdeTerminalForPath,
  pushIdeNavLocation,
  putIdeLocalHistoryLabel,
  revealInIdeProjectTree,
  toggleIdeTabPinned,
} from './ideEditorStoreBridge.svelte';
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
      hint: ide.relativePath(t.path),
      action: () => {
        if (ide.activePath && ide.activePath !== t.path) flushAutoLocalHistorySnapshot(ide.activePath);
        ide.activePath = t.path;
      },
    };
  });
  items.push(
    { separator: true },
    { label: 'Close unmodified tabs', action: () => void closeUnmodifiedIdeTabs() },
    { label: 'Close all tabs', icon: 'x', action: () => void closeAllIdeTabs() },
  );
  ctxMenu.showAt(anchor, items, { align: 'end' });
}

function stripMacPrivatePrefix(p: string): string {
  return p.startsWith('/private/var/') || p.startsWith('/private/tmp/') ? p.slice('/private'.length) : p;
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
    const sameFile = stripMacPrivatePrefix(ide.resolvePath(target.path)) === stripMacPrivatePrefix(ide.resolvePath(fromPath));
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

interface BlameEntry { start_line?: number; end_line?: number; line?: number; author?: string; summary?: string; commit?: string; date?: string; }

export async function runFetchIdeGitBlame(rootPath: string, filePath: string, signal?: AbortSignal): Promise<Record<number, string> | null> {
  try {
    const repos = await api.bg.get<Array<{ id: string; path: string }>>('/repos');
    const matches = (repos ?? [])
      .map((r) => ({ id: r.id, root: r.path.replace(/\/+$/, '') }))
      .filter((r) => filePath === r.root || filePath.startsWith(`${r.root}/`))
      .sort((a, b) => b.root.length - a.root.length);
    if (!matches.length && (!rootPath || !filePath.startsWith(rootPath))) return null;
    if (!matches.length) return null;
    const best = matches[0];
    const relPath = filePath.slice(best.root.length).replace(/^\/+/, '');
    if (!relPath) return null;
    const resp = await api.bg.get<{ lines?: BlameEntry[]; hunks?: BlameEntry[] } | BlameEntry[]>(
      `/repos/${encodeURIComponent(best.id)}/blame?path=${encodeURIComponent(relPath)}`,
      signal,
    );
    const list = Array.isArray(resp) ? resp : (resp.lines ?? resp.hunks ?? []);
    if (!list.length) return null;
    const map: Record<number, string> = {};
    for (const item of list) {
      const sha = item.commit ? item.commit.slice(0, 7) : '';
      const label = [item.author, sha, item.summary].filter(Boolean).join(' · ').slice(0, 56);
      if (!label) continue;
      if (item.line != null) map[item.line] = label;
      else if (item.start_line != null) map[item.start_line] = label;
    }
    return Object.keys(map).length > 0 ? map : null;
  } catch {
    return null;
  }
}
