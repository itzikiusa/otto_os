// Typed bridge + navigation/dive/blame/local-history/git helpers between
// `IdeEditorArea` and `ideStore.svelte.ts`.

import { idePreferences } from '../../../lib/idePreferences.svelte';
import { ide, type IdeOpenTab } from '../ideStore.svelte';

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

export function flushAutoLocalHistorySnapshot(filePath?: string): void {
  ide.flushHistorySnapshot(filePath);
}

export async function putIdeLocalHistoryLabel(filePath: string, content: string): Promise<void> {
  const folders = getIdeProjectFolders();
  const root = folders.find((f) => filePath === f || filePath.startsWith(`${f}/`)) ?? ide.target.root;
  const { promptPutLocalHistoryLabel } = await import('../history/ideComparisonLoader');
  await promptPutLocalHistoryLabel(root, filePath, { content });
}

export const ideEditorPrefs = {
  get formatOnSave(): boolean {
    return idePreferences.current.formatOnSave;
  },
  set formatOnSave(v: boolean) {
    idePreferences.update({ formatOnSave: Boolean(v) });
  },
};

export function toggleFormatOnSave(): void {
  const next = !idePreferences.current.formatOnSave;
  ide.setFormatOnSave(next);
}

export function isIdeTabPinned(tab: IdeOpenTab): boolean {
  return Boolean(tab.pinned);
}

export function toggleIdeTabPinned(path: string): void {
  ide.togglePinTab(path);
}

export async function closeAllIdeTabs(): Promise<void> {
  flushAutoLocalHistorySnapshot();
  await ide.closeAllTabs();
}

export async function closeUnmodifiedIdeTabs(): Promise<void> {
  ide.closeUnmodifiedTabs();
}

export async function revealInIdeProjectTree(path: string): Promise<void> {
  await ide.revealInProjectTree(path);
}

export async function openIdeTerminalForPath(path: string): Promise<void> {
  ide.openTerminalAt(path);
}

export function areIdeBreakpointsMuted(): boolean {
  return Boolean(ide.breakpointsMuted);
}

export function toggleIdeBreakpointsMuted(): void {
  ide.toggleMuteBreakpoints();
}

export function pushIdeNavLocation(path: string, line: number, col: number): void {
  if (!path) return;
  ide.pushNavLocation(path, line, col);
}

export function canIdeNavigateBack(): boolean {
  return Boolean(ide.canNavigateBack);
}

export function canIdeNavigateForward(): boolean {
  return Boolean(ide.canNavigateForward);
}

export async function navigateIdeBack(): Promise<void> {
  await ide.navigateBack();
}

export async function navigateIdeForward(): Promise<void> {
  await ide.navigateForward();
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

  if (tab.external || (!inWorkspace && allRoots.length > 0)) return { external: true, sdkLabel: 'External' };
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
