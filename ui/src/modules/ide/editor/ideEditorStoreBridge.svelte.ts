// Typed bridge + zero-leak navigation/dive/blame helpers between `IdeEditorArea`
// and `ideStore.svelte.ts` (compatible both standalone and merged with `@ui-shell-run`).

import { api } from '../../../lib/api/client';
import { ctxMenu } from '../../../lib/contextmenu.svelte';
import { ide, ideBasename, type IdeOpenTab } from '../ideStore.svelte';
import type { IdeReferenceLocation } from '../store/ideTypes';
import { resolveLocalFastDive } from './ideLangAnalyzer';

interface NavEntry {
  path: string;
  line: number;
  col: number;
}

// Optional extended surface added by `@ui-shell-run` on `ideStore.svelte.ts`.
interface IdeStoreExtended {
  breakpointsMuted?: boolean;
  toggleMuteBreakpoints?: () => void;
  togglePinTab?: (path: string) => void;
  closeAllTabs?: () => Promise<void> | void;
  closeUnmodifiedTabs?: () => Promise<void> | void;
  revealInProjectTree?: (path: string) => Promise<void> | void;
  openTerminalAt?: (path: string) => Promise<void> | void;
  pushNavLocation?: (path: string, line: number, col: number) => void;
  navigateBack?: () => Promise<void> | void;
  navigateForward?: () => Promise<void> | void;
  canNavigateBack?: boolean;
  canNavigateForward?: boolean;
  openFindInPath?: (opts?: { replace?: boolean; scopePath?: string | null; seedQuery?: string }) => void;
  searchScopePath?: string | null;
  searchReplaceMode?: boolean;
}

const extIde = (): IdeStoreExtended => ide as unknown as IdeStoreExtended;

// Bounded fallback state when running before `@ui-shell-run` merges; cleared on workspace reset.
const MAX_NAV_STACK = 60;
let fallbackPinned = $state<Record<string, boolean>>({});
let fallbackMuted = $state(false);
let fallbackBackStack = $state<NavEntry[]>([]);
let fallbackFwdStack = $state<NavEntry[]>([]);
let navJumping = false;

const FORMAT_ON_SAVE_KEY = 'otto.ide.formatOnSave';
function readBoolPref(key: string): boolean {
  try { return typeof localStorage !== 'undefined' && localStorage.getItem(key) === '1'; }
  catch { return false; }
}
export const ideEditorPrefs = $state({
  formatOnSave: readBoolPref(FORMAT_ON_SAVE_KEY),
});
export function toggleFormatOnSave(): void {
  const next = !ideEditorPrefs.formatOnSave;
  ideEditorPrefs.formatOnSave = next;
  const s = extIde() as { setFormatOnSave?: (v: boolean) => void };
  if (typeof s.setFormatOnSave === 'function') s.setFormatOnSave(next);
  try { localStorage.setItem(FORMAT_ON_SAVE_KEY, next ? '1' : '0'); }
  catch { /* storage unavailable */ }
}

/** Reset all local fallback state when closing all tabs or switching workspace (`0` retained paths). */
export function resetIdeEditorBridgeState(): void {
  fallbackPinned = {};
  fallbackBackStack = [];
  fallbackFwdStack = [];
}

// ── Tab Pinning & Bulk Tab Actions ───────────────────────────────────────────

export function isIdeTabPinned(tab: IdeOpenTab): boolean {
  const rec = tab as IdeOpenTab & { pinned?: boolean };
  return Boolean(rec.pinned ?? fallbackPinned[tab.path]);
}

export function toggleIdeTabPinned(path: string): void {
  const s = extIde();
  if (typeof s.togglePinTab === 'function') {
    s.togglePinTab(path);
    return;
  }
  const next = !fallbackPinned[path];
  if (next) fallbackPinned = { ...fallbackPinned, [path]: true };
  else {
    const copy = { ...fallbackPinned };
    delete copy[path];
    fallbackPinned = copy;
  }
  ide.pinTab(path);
}

export async function closeAllIdeTabs(): Promise<void> {
  const s = extIde();
  if (typeof s.closeAllTabs === 'function') {
    await s.closeAllTabs();
    resetIdeEditorBridgeState();
    return;
  }
  for (const t of [...ide.tabs]) {
    await ide.closeTab(t.path);
  }
  if (ide.tabs.length === 0) resetIdeEditorBridgeState();
}

export async function closeUnmodifiedIdeTabs(): Promise<void> {
  const s = extIde();
  if (typeof s.closeUnmodifiedTabs === 'function') {
    await s.closeUnmodifiedTabs();
    return;
  }
  for (const t of [...ide.tabs]) {
    const dirty = t.dirty ?? (t.content !== t.savedContent);
    if (!dirty && !isIdeTabPinned(t)) {
      await ide.closeTab(t.path);
    }
  }
}

export async function revealInIdeProjectTree(path: string): Promise<void> {
  const s = extIde() as Record<string, unknown>;
  if (typeof s.revealInProjectTree === 'function') {
    await (s.revealInProjectTree as (p: string) => Promise<void>)(path);
    return;
  }
  if ('leftSidebarOpen' in s) s.leftSidebarOpen = true;
  const rel = ide.relativePath(path);
  if (!rel || rel.startsWith('..')) return;
  const parts = rel.split('/').filter(Boolean);
  let acc = ide.target.root.replace(/\/+$/, '');
  const nextExpanded = { ...ide.expandedDirs };
  for (let i = 0; i < parts.length - 1; i++) {
    acc = `${acc}/${parts[i]}`;
    nextExpanded[acc] = true;
    if (!ide.childrenByDir[acc]) void ide.loadTree(acc);
  }
  ide.expandedDirs = nextExpanded;
}

export async function openIdeTerminalForPath(path: string): Promise<void> {
  const s = extIde() as Record<string, unknown>;
  if (typeof s.openTerminalAt === 'function') {
    await (s.openTerminalAt as (p: string) => Promise<void>)(path);
    return;
  }
  ide.openBottomTab('terminal');
  if (typeof s.openWorkspaceTerminal === 'function') {
    await (s.openWorkspaceTerminal as () => Promise<void>)();
  }
}

// ── Breakpoint Mute State ────────────────────────────────────────────────────

export function areIdeBreakpointsMuted(): boolean {
  const s = extIde();
  return s.breakpointsMuted !== undefined ? Boolean(s.breakpointsMuted) : fallbackMuted;
}

export function toggleIdeBreakpointsMuted(): void {
  const s = extIde();
  if (typeof s.toggleMuteBreakpoints === 'function') {
    s.toggleMuteBreakpoints();
    return;
  }
  fallbackMuted = !fallbackMuted;
  void ide.syncBreakpointsToSession();
}

// ── Back / Forward Cursor Navigation History (`⌘[` / `⌘]`) ──────────────────

export function pushIdeNavLocation(path: string, line: number, col: number): void {
  if (!path || navJumping) return;
  const s = extIde();
  if (typeof s.pushNavLocation === 'function') {
    s.pushNavLocation(path, line, col);
    return;
  }
  const top = fallbackBackStack[fallbackBackStack.length - 1];
  if (top && top.path === path && Math.abs(top.line - line) <= 1) return;
  fallbackBackStack = [...fallbackBackStack.slice(-(MAX_NAV_STACK - 1)), { path, line, col }];
  fallbackFwdStack = [];
}

export function canIdeNavigateBack(): boolean {
  const s = extIde();
  return s.canNavigateBack !== undefined ? Boolean(s.canNavigateBack) : fallbackBackStack.length > 0;
}

export function canIdeNavigateForward(): boolean {
  const s = extIde();
  return s.canNavigateForward !== undefined ? Boolean(s.canNavigateForward) : fallbackFwdStack.length > 0;
}

export async function navigateIdeBack(): Promise<void> {
  const s = extIde();
  if (typeof s.navigateBack === 'function') {
    await s.navigateBack();
    return;
  }
  const target = fallbackBackStack[fallbackBackStack.length - 1];
  if (!target) return;
  fallbackBackStack = fallbackBackStack.slice(0, -1);
  if (ide.activePath) {
    fallbackFwdStack = [...fallbackFwdStack.slice(-(MAX_NAV_STACK - 1)), {
      path: ide.activePath, line: ide.cursorInfo.line, col: ide.cursorInfo.col,
    }];
  }
  navJumping = true;
  try {
    await ide.openFile(target.path, { line: target.line, col: target.col, preview: false });
  } finally {
    navJumping = false;
  }
}

export async function navigateIdeForward(): Promise<void> {
  const s = extIde();
  if (typeof s.navigateForward === 'function') {
    await s.navigateForward();
    return;
  }
  const target = fallbackFwdStack[fallbackFwdStack.length - 1];
  if (!target) return;
  fallbackFwdStack = fallbackFwdStack.slice(0, -1);
  if (ide.activePath) {
    fallbackBackStack = [...fallbackBackStack.slice(-(MAX_NAV_STACK - 1)), {
      path: ide.activePath, line: ide.cursorInfo.line, col: ide.cursorInfo.col,
    }];
  }
  navJumping = true;
  try {
    await ide.openFile(target.path, { line: target.line, col: target.col, preview: false });
  } finally {
    navJumping = false;
  }
}

// ── External / SDK Read-Only File Classification ─────────────────────────────

export function classifyIdeFile(tab: IdeOpenTab, rootPath: string, additionalRoots: string[] = []): {
  external: boolean;
  sdkLabel: string | null;
} {
  const p = tab.path;
  if (p.startsWith('jdt://') || p.startsWith('jar:')) return { external: true, sdkLabel: 'JDK / JAR' };
  if (p.includes('/pkg/mod/') || p.includes('/go/pkg/mod/')) return { external: true, sdkLabel: 'Go module' };
  if (/\/go(?:\d[\w.]*)?\/(?:libexec\/)?src\//.test(p) || p.includes('/goroot/')) return { external: true, sdkLabel: 'Go SDK' };
  if (p.includes('/node_modules/')) return { external: true, sdkLabel: 'node_modules' };
  if (p.includes('/.cargo/registry/') || p.includes('/rustlib/src/')) return { external: true, sdkLabel: 'Rust crate' };
  if (p.includes('/site-packages/') || p.includes('/dist-packages/')) return { external: true, sdkLabel: 'Python pkg' };
  if (p.includes('/.otto/toolchains/')) return { external: true, sdkLabel: 'SDK' };

  const allRoots = [rootPath, ...additionalRoots].map((r) => r.replace(/\/+$/, '')).filter(Boolean);
  const inWorkspace = allRoots.some((r) => p === r || p.startsWith(`${r}/`));
  const tabExt = (tab as IdeOpenTab & { external?: boolean }).external;
  if (tabExt || (!inWorkspace && allRoots.length > 0)) {
    return { external: true, sdkLabel: 'External' };
  }
  return { external: false, sdkLabel: null };
}

// ── Full Go-to-Definition & Go-to-Implementation Dive-In ────────────────────

function stripMacPrivatePrefix(p: string): string {
  return p.startsWith('/private/var/') || p.startsWith('/private/tmp/') ? p.slice('/private'.length) : p;
}

function isImportBindingLine(lineText: string): boolean {
  return /^\s*(?:import\b|(?:const|let|var)\s+.*=\s*require\()/.test(lineText);
}

export async function performIdeDive(
  mode: 'definition' | 'implementation',
  fromPath: string,
  target: { path?: string; line: number; column: number; symbol?: string; originLine?: number; originColumn?: number },
  content: string,
): Promise<void> {
  const originLine = target.originLine ?? (ide.cursorInfo.line || target.line);
  const originCol = target.originColumn ?? (ide.cursorInfo.col || target.column);
  pushIdeNavLocation(fromPath, originLine, originCol);

  // 1. Direct path already resolved by live LSP client — except when TS/JS LSP resolves
  // a symbol to a same-file `import` / `require(` binding line instead of `node_modules` `.d.ts`.
  let sameFileImportFallback: { path: string; line: number; column: number } | null = null;
  if (target.path) {
    const sameFile =
      stripMacPrivatePrefix(ide.resolvePath(target.path)) === stripMacPrivatePrefix(ide.resolvePath(fromPath));
    const destLineText = sameFile ? (content.split(/\r?\n/)[target.line - 1] ?? '') : '';
    if (mode === 'definition' && sameFile && isImportBindingLine(destLineText)) {
      sameFileImportFallback = { path: target.path, line: target.line, column: target.column };
    } else {
      await ide.openFile(target.path, { line: target.line, col: target.column, preview: false });
      pushIdeNavLocation(target.path, target.line, target.column);
      return;
    }
  }

  // 2. Instant <1ms local resolver for Shell functions/source, JSON $ref, and relative file paths.
  if (mode === 'definition') {
    const localHit = resolveLocalFastDive(fromPath, ide.target.root, content, originLine, target.symbol);
    if (localHit) {
      await ide.openFile(localHit.path, { line: localHit.line, col: localHit.column, preview: false });
      pushIdeNavLocation(localHit.path, localHit.line, localHit.column);
      return;
    }
  }

  // 3. Backend LSP / lexical definition & implementation endpoint (`/ide/lsp/definition` | `/ide/lsp/implementation`).
  const endpoint = mode === 'implementation' ? '/ide/lsp/implementation' : '/ide/lsp/definition';
  try {
    const resp = await api.post<{ symbol: string; locations: IdeReferenceLocation[] }>(endpoint, {
      target: ide.targetPayload,
      path: fromPath,
      line: originLine,
      column: originCol,
      symbol: target.symbol,
      content,
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
  } catch {
    /* Fall back below */
  }

  if (sameFileImportFallback) {
    await ide.openFile(sameFileImportFallback.path, {
      line: sameFileImportFallback.line,
      col: sameFileImportFallback.column,
      preview: false,
    });
    pushIdeNavLocation(sameFileImportFallback.path, sameFileImportFallback.line, sameFileImportFallback.column);
    return;
  }

  // 4. Final fallback: Find Usages dock.
  await ide.findReferences(fromPath, originLine, originCol, target.symbol);
}

// ── On-Demand Git Blame Fetcher (Zero Retained Cache When Off) ───────────────

interface BlameEntry {
  start_line?: number;
  end_line?: number;
  line?: number;
  author?: string;
  summary?: string;
  commit?: string;
  date?: string;
}

export async function fetchIdeGitBlame(
  rootPath: string,
  filePath: string,
  signal?: AbortSignal,
): Promise<Record<number, string> | null> {
  if (!rootPath || !filePath.startsWith(rootPath)) return null;
  try {
    const repos = await api.bg.get<Array<{ id: string; path: string }>>('/repos', signal);
    const cleanRoot = rootPath.replace(/\/+$/, '');
    const repo = (repos ?? []).find((r) => {
      const rp = r.path.replace(/\/+$/, '');
      return cleanRoot === rp || cleanRoot.startsWith(`${rp}/`);
    });
    if (!repo) return null;
    const repoRoot = repo.path.replace(/\/+$/, '');
    const relPath = filePath.slice(repoRoot.length).replace(/^\/+/, '');
    if (!relPath) return null;
    const resp = await api.bg.get<{ lines?: BlameEntry[]; hunks?: BlameEntry[] } | BlameEntry[]>(
      `/repos/${encodeURIComponent(repo.id)}/blame?path=${encodeURIComponent(relPath)}`,
      signal,
    );
    const list = Array.isArray(resp) ? resp : (resp.lines ?? resp.hunks ?? []);
    if (list.length === 0) return null;
    const map: Record<number, string> = {};
    for (const item of list) {
      const sha = item.commit ? item.commit.slice(0, 7) : '';
      const label = [item.author, sha, item.summary].filter(Boolean).join(' · ').slice(0, 56);
      if (!label) continue;
      if (item.line != null) {
        map[item.line] = label;
      } else if (item.start_line != null) {
        map[item.start_line] = label;
      }
    }
    return Object.keys(map).length > 0 ? map : null;
  } catch {
    return null;
  }
}
