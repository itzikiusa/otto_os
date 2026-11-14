// Typed bridge + zero-leak navigation/dive/blame/local-history/git helpers between
// `IdeEditorArea` and `ideStore.svelte.ts`.

import { api } from '../../../lib/api/client';
import { confirmer } from '../../../lib/confirm.svelte';
import { toasts } from '../../../lib/toast.svelte';
import { ide, ideBasename, type IdeOpenTab } from '../ideStore.svelte';

interface NavEntry { path: string; line: number; col: number; }

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
  additionalRoots?: string[];
}

const extIde = (): IdeStoreExtended => ide as unknown as IdeStoreExtended;

export function getIdeProjectFolders(): string[] {
  const roots = ide.projectRoots.map((r) => r.path);
  const extra = ide.additionalRoots ?? ide.target.additional_roots ?? [];
  return [...new Set([ide.target.root, ...roots, ...extra].map((r) => r.replace(/\/+$/, '')).filter(Boolean))];
}

/** Smart middle-ellipsis + parent-folder disambiguation for GoLand-parity tabs (`CLS = 0`). */
export function formatIdeTabDisplayName(
  tab: Pick<IdeOpenTab, 'path' | 'name'>,
  allTabs: ReadonlyArray<Pick<IdeOpenTab, 'path' | 'name'>>,
  maxChars = 28,
): string {
  const sameName = allTabs.filter((t) => t.name === tab.name && t.path !== tab.path);
  let base = tab.name;
  if (sameName.length > 0) {
    const clean = tab.path.split('?')[0].replace(/\/+$/, '');
    const segs = clean.split('/').filter(Boolean);
    const parent = segs.length >= 2 ? segs[segs.length - 2] : '';
    if (parent) base = `${parent}/${tab.name}`;
  }
  if (base.length <= maxChars) return base;
  const dot = base.lastIndexOf('.');
  const suffixLen = dot > 0 && base.length - dot <= 12 ? Math.min(13, base.length - dot + 5) : Math.floor((maxChars - 1) / 2);
  const prefixLen = Math.max(6, maxChars - 1 - suffixLen);
  return `${base.slice(0, prefixLen)}…${base.slice(base.length - suffixLen)}`;
}

// ── Auto-Snapshotting Local History (`4s` idle + tab switch / blur) ──────────

const snapshotTimers = new Map<string, ReturnType<typeof setTimeout>>();
const pendingSnapshots = new Map<string, string>();
const lastSnapshotted = new Map<string, string>();

async function postHistorySnapshot(filePath: string, content: string, label: string): Promise<boolean> {
  if (!filePath || filePath.includes('://')) return false;
  if (label === 'Change' && lastSnapshotted.get(filePath) === content) return false;
  lastSnapshotted.set(filePath, content);
  const folders = getIdeProjectFolders();
  const root = folders.find((f) => filePath === f || filePath.startsWith(`${f}/`)) ?? ide.target.root;
  try {
    await api.bg.post('/ide/history/snapshot', {
      target: { ...ide.targetPayload, root, additional_roots: folders.filter((f) => f !== root) },
      path: filePath,
      content,
      label,
    });
    return true;
  } catch {
    return false;
  }
}

export function scheduleAutoLocalHistorySnapshot(filePath: string, content: string): void {
  if (!filePath || filePath.includes('://')) return;
  pendingSnapshots.set(filePath, content);
  const prev = snapshotTimers.get(filePath);
  if (prev !== undefined) clearTimeout(prev);
  snapshotTimers.set(
    filePath,
    setTimeout(() => {
      snapshotTimers.delete(filePath);
      const latest = pendingSnapshots.get(filePath);
      pendingSnapshots.delete(filePath);
      if (latest !== undefined) void postHistorySnapshot(filePath, latest, 'Change');
    }, 4000),
  );
}

export function flushAutoLocalHistorySnapshot(filePath?: string): void {
  const keys = filePath ? [filePath] : [...pendingSnapshots.keys()];
  for (const k of keys) {
    const t = snapshotTimers.get(k);
    if (t !== undefined) { clearTimeout(t); snapshotTimers.delete(k); }
    const latest = pendingSnapshots.get(k);
    pendingSnapshots.delete(k);
    if (latest !== undefined) void postHistorySnapshot(k, latest, 'Change');
  }
}

export async function putIdeLocalHistoryLabel(filePath: string, content: string): Promise<void> {
  const label = await confirmer.promptText(
    `Create a named Local History label for ${ideBasename(filePath)}:`,
    { title: 'Put Local History Label', confirmLabel: 'Put Label', placeholder: 'e.g. Before refactoring handler', danger: false },
  );
  if (!label) return;
  const ok = await postHistorySnapshot(filePath, content, label);
  if (ok) toasts.success('Local History label saved', label);
  else toasts.info('Saved label locally', label);
}

// ── Bounded Fallback State & Navigation Stack ────────────────────────────────

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
export const ideEditorPrefs = $state({ formatOnSave: readBoolPref(FORMAT_ON_SAVE_KEY) });
export function toggleFormatOnSave(): void {
  const next = !ideEditorPrefs.formatOnSave;
  ideEditorPrefs.formatOnSave = next;
  const s = extIde() as { setFormatOnSave?: (v: boolean) => void };
  if (typeof s.setFormatOnSave === 'function') s.setFormatOnSave(next);
  try { localStorage.setItem(FORMAT_ON_SAVE_KEY, next ? '1' : '0'); } catch { /* ignore */ }
}

export function evictIdeEditorBridgePaths(paths: Iterable<string>): void {
  for (const p of paths) {
    const t = snapshotTimers.get(p);
    if (t !== undefined) { clearTimeout(t); snapshotTimers.delete(p); }
    pendingSnapshots.delete(p);
    lastSnapshotted.delete(p);
    delete fallbackPinned[p];
  }
}

export function resetIdeEditorBridgeState(): void {
  flushAutoLocalHistorySnapshot();
  for (const t of snapshotTimers.values()) clearTimeout(t);
  snapshotTimers.clear();
  pendingSnapshots.clear();
  lastSnapshotted.clear();
  fallbackPinned = {};
  fallbackBackStack = [];
  fallbackFwdStack = [];
}

export function isIdeTabPinned(tab: IdeOpenTab): boolean {
  const rec = tab as IdeOpenTab & { pinned?: boolean };
  return Boolean(rec.pinned ?? fallbackPinned[tab.path]);
}

export function toggleIdeTabPinned(path: string): void {
  const s = extIde();
  if (typeof s.togglePinTab === 'function') { s.togglePinTab(path); return; }
  const next = !fallbackPinned[path];
  if (next) fallbackPinned = { ...fallbackPinned, [path]: true };
  else { const copy = { ...fallbackPinned }; delete copy[path]; fallbackPinned = copy; }
  ide.pinTab(path);
}

export async function closeAllIdeTabs(): Promise<void> {
  flushAutoLocalHistorySnapshot();
  const s = extIde();
  if (typeof s.closeAllTabs === 'function') { await s.closeAllTabs(); resetIdeEditorBridgeState(); return; }
  for (const t of [...ide.tabs]) await ide.closeTab(t.path);
  if (ide.tabs.length === 0) resetIdeEditorBridgeState();
}

export async function closeUnmodifiedIdeTabs(): Promise<void> {
  const s = extIde();
  if (typeof s.closeUnmodifiedTabs === 'function') { await s.closeUnmodifiedTabs(); return; }
  for (const t of [...ide.tabs]) {
    const dirty = t.dirty ?? (t.content !== t.savedContent);
    if (!dirty && !isIdeTabPinned(t)) await ide.closeTab(t.path);
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
  if (typeof s.openWorkspaceTerminal === 'function') await (s.openWorkspaceTerminal as () => Promise<void>)();
}

export function areIdeBreakpointsMuted(): boolean {
  const s = extIde();
  return s.breakpointsMuted !== undefined ? Boolean(s.breakpointsMuted) : fallbackMuted;
}

export function toggleIdeBreakpointsMuted(): void {
  const s = extIde();
  if (typeof s.toggleMuteBreakpoints === 'function') { s.toggleMuteBreakpoints(); return; }
  fallbackMuted = !fallbackMuted;
  void ide.syncBreakpointsToSession();
}

export function pushIdeNavLocation(path: string, line: number, col: number): void {
  if (!path || navJumping) return;
  const s = extIde();
  if (typeof s.pushNavLocation === 'function') { s.pushNavLocation(path, line, col); return; }
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
  if (typeof s.navigateBack === 'function') { await s.navigateBack(); return; }
  const target = fallbackBackStack[fallbackBackStack.length - 1];
  if (!target) return;
  fallbackBackStack = fallbackBackStack.slice(0, -1);
  if (ide.activePath) fallbackFwdStack = [...fallbackFwdStack.slice(-(MAX_NAV_STACK - 1)), { path: ide.activePath, line: ide.cursorInfo.line, col: ide.cursorInfo.col }];
  navJumping = true;
  try { await ide.openFile(target.path, { line: target.line, col: target.col, preview: false }); }
  finally { navJumping = false; }
}

export async function navigateIdeForward(): Promise<void> {
  const s = extIde();
  if (typeof s.navigateForward === 'function') { await s.navigateForward(); return; }
  const target = fallbackFwdStack[fallbackFwdStack.length - 1];
  if (!target) return;
  fallbackFwdStack = fallbackFwdStack.slice(0, -1);
  if (ide.activePath) fallbackBackStack = [...fallbackBackStack.slice(-(MAX_NAV_STACK - 1)), { path: ide.activePath, line: ide.cursorInfo.line, col: ide.cursorInfo.col }];
  navJumping = true;
  try { await ide.openFile(target.path, { line: target.line, col: target.col, preview: false }); }
  finally { navJumping = false; }
}

// ── External / SDK Read-Only File Classification ─────────────────────────────

export function classifyIdeFile(tab: IdeOpenTab, rootPath: string, additionalRoots: string[] = []): {
  external: boolean;
  sdkLabel: string | null;
} {
  const p = tab.path;
  const allRoots = [rootPath, ...additionalRoots].map((r) => r.replace(/\/+$/, '')).filter(Boolean);
  const inWorkspace = allRoots.some((r) => p === r || p.startsWith(`${r}/`));

  if (p.startsWith('jdt://') || p.startsWith('jar:')) return { external: true, sdkLabel: 'JDK / JAR' };
  if (p.includes('/.m2/repository/') || p.includes('/.gradle/caches/')) return { external: true, sdkLabel: 'Maven / Gradle' };
  if (p.includes('/pkg/mod/') || p.includes('/go/pkg/mod/')) return { external: true, sdkLabel: 'Go module' };
  if (!inWorkspace && (/\/go(?:@[\w.-]+)?(?:\/[\w.-]+|\d[\w.-]*)?\/(?:libexec\/)?src\//.test(p) || p.includes('/goroot/'))) {
    return { external: true, sdkLabel: 'Go SDK' };
  }
  if (p.includes('/node_modules/')) return { external: true, sdkLabel: 'node_modules' };
  if (p.includes('/.cargo/registry/') || p.includes('/rustlib/src/')) return { external: true, sdkLabel: 'Rust crate' };
  if (p.includes('/site-packages/') || p.includes('/dist-packages/')) return { external: true, sdkLabel: 'Python pkg' };
  if (p.includes('/.otto/toolchains/')) return { external: true, sdkLabel: 'SDK' };

  const tabExt = (tab as IdeOpenTab & { external?: boolean }).external;
  if (tabExt || (!inWorkspace && allRoots.length > 0)) return { external: true, sdkLabel: 'External' };
  return { external: false, sdkLabel: null };
}

export function performIdeDive(
  mode: 'definition' | 'implementation',
  fromPath: string,
  target: { path?: string; line: number; column: number; symbol?: string; originLine?: number; originColumn?: number },
  content: string,
): Promise<void> {
  return import('./ideEditorActionsLazy').then((m) => m.runPerformIdeDive(mode, fromPath, target, content));
}

export function fetchIdeGitBlame(rootPath: string, filePath: string, signal?: AbortSignal): Promise<Record<number, string> | null> {
  return import('./ideEditorActionsLazy').then((m) => m.runFetchIdeGitBlame(rootPath, filePath, signal));
}
