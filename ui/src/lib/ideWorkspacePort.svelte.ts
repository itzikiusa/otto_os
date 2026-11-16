// Reactive live IDE workspace snapshot port shared between `modules/ide` and `modules/threads`
// without cross-module imports or window global acoplings.

import { untrack } from 'svelte';
import { normalizeProjectPath } from './ideProjectsModel';
import { lsGet, lsRemove, lsSet } from './storage';

export interface IdeOpenTabSnapshot {
  path: string;
  dirty: boolean;
}

export interface IdeLiveWorkspaceSnapshot {
  root: string | null;
  additionalRoots: string[];
  activePath: string | null;
  tabs: IdeOpenTabSnapshot[];
}

const LAST_ROOT_KEY = 'otto.ide.lastRoot';
const AGENT_OPEN_KEY = 'otto.ide.agentOpen';
const ROOTS_LOCAL_PREFIX = 'otto.ide.roots:local:';
const IDE_CONTEXT_CHANGED_EVENT = 'otto:ide-context-changed';
const IDE_PROJECTS_CHANGED_EVENT = 'otto:ide-projects-changed';

function cleanRootOrNull(raw: string | null | undefined): string | null {
  const clean = normalizeProjectPath(raw ?? '');
  return clean && clean !== '~' ? clean : null;
}

function parseRootsList(raw: string | null, excludeCanon: string): string[] {
  if (!raw) return [];
  try {
    const parsed: unknown = JSON.parse(raw);
    if (!Array.isArray(parsed)) return [];
    const out: string[] = [];
    for (let i = 0; i < parsed.length; i++) {
      if (typeof parsed[i] !== 'string') continue;
      const clean = normalizeProjectPath(parsed[i] as string);
      if (clean && clean !== '~' && clean !== excludeCanon && !out.includes(clean)) {
        out.push(clean);
      }
    }
    return out;
  } catch {
    return [];
  }
}

function snapshotsEqual(a: IdeLiveWorkspaceSnapshot, b: IdeLiveWorkspaceSnapshot): boolean {
  if (a.root !== b.root || a.activePath !== b.activePath) return false;
  if (a.additionalRoots.length !== b.additionalRoots.length) return false;
  if (a.tabs.length !== b.tabs.length) return false;
  for (let i = 0; i < a.additionalRoots.length; i++) {
    if (a.additionalRoots[i] !== b.additionalRoots[i]) return false;
  }
  for (let i = 0; i < a.tabs.length; i++) {
    if (a.tabs[i].path !== b.tabs[i].path || a.tabs[i].dirty !== b.tabs[i].dirty) return false;
  }
  return true;
}

class IdeWorkspacePort {
  current = $state<IdeLiveWorkspaceSnapshot>({
    root: null,
    additionalRoots: [],
    activePath: null,
    tabs: [],
  });
  lastRoot = $state<string | null>(cleanRootOrNull(lsGet(LAST_ROOT_KEY)));
  agentDockOpen = $state<boolean>(lsGet(AGENT_OPEN_KEY) === '1');
  private rootsRev = $state(0);
  private rootsCache = new Map<string, string[]>();

  constructor() {
    if (typeof window !== 'undefined') {
      window.addEventListener('storage', (e) => {
        if (!e.key || e.key === LAST_ROOT_KEY) {
          this.lastRoot = cleanRootOrNull(lsGet(LAST_ROOT_KEY));
        }
        if (!e.key || e.key === AGENT_OPEN_KEY) {
          this.agentDockOpen = lsGet(AGENT_OPEN_KEY) === '1';
        }
        if (!e.key || e.key.startsWith(ROOTS_LOCAL_PREFIX)) {
          this.rootsCache.clear();
          this.rootsRev = untrack(() => this.rootsRev) + 1;
        }
      });
      window.addEventListener('otto:ide-storage-cleared', () => {
        this.current = { root: null, additionalRoots: [], activePath: null, tabs: [] };
        this.lastRoot = null;
        this.agentDockOpen = false;
        this.rootsCache.clear();
        this.rootsRev = untrack(() => this.rootsRev) + 1;
      });
    }
  }

  publish(next: IdeLiveWorkspaceSnapshot): void {
    const normalized: IdeLiveWorkspaceSnapshot = {
      root: cleanRootOrNull(next.root),
      additionalRoots: next.additionalRoots
        .map((r) => normalizeProjectPath(r))
        .filter((r) => Boolean(r && r !== '~')),
      activePath: next.activePath ? normalizeProjectPath(next.activePath) : null,
      tabs: next.tabs.map((t) => ({ path: t.path, dirty: Boolean(t.dirty) })),
    };
    const prev = untrack(() => this.current);
    if (snapshotsEqual(prev, normalized)) return;
    this.current = normalized;
    if (typeof window !== 'undefined') {
      window.dispatchEvent(new CustomEvent(IDE_CONTEXT_CHANGED_EVENT, { detail: normalized }));
    }
  }

  setLastRoot(root: string | null): void {
    const clean = cleanRootOrNull(root);
    if (untrack(() => this.lastRoot) === clean && lsGet(LAST_ROOT_KEY) === (clean ?? null)) return;
    this.lastRoot = clean;
    if (clean) lsSet(LAST_ROOT_KEY, clean);
    else lsRemove(LAST_ROOT_KEY);
    if (typeof window !== 'undefined') {
      window.dispatchEvent(new CustomEvent(IDE_PROJECTS_CHANGED_EVENT, { detail: { root: clean } }));
    }
  }

  setAgentDockOpen(open: boolean): void {
    const next = Boolean(open);
    this.agentDockOpen = next;
    lsSet(AGENT_OPEN_KEY, next ? '1' : '0');
  }

  rootsForLocal(root: string): string[] {
    void this.rootsRev;
    const canon = cleanRootOrNull(root);
    if (!canon) return [];
    const cached = this.rootsCache.get(canon);
    if (cached) return [...cached];
    const trimmed = root.trim().replace(/\/+$/, '');
    const raw =
      lsGet(`${ROOTS_LOCAL_PREFIX}${canon}`) ??
      (trimmed && trimmed !== canon ? lsGet(`${ROOTS_LOCAL_PREFIX}${trimmed}`) : null);
    const parsed = parseRootsList(raw, canon);
    this.rootsCache.set(canon, parsed);
    return [...parsed];
  }

  saveRootsForLocal(root: string, roots: string[]): void {
    const canon = cleanRootOrNull(root);
    if (!canon) return;
    const deduped: string[] = [];
    for (let i = 0; i < roots.length; i++) {
      const clean = normalizeProjectPath(roots[i]);
      if (clean && clean !== '~' && clean !== canon && !deduped.includes(clean)) {
        deduped.push(clean);
      }
    }
    this.rootsCache.set(canon, deduped);
    lsSet(`${ROOTS_LOCAL_PREFIX}${canon}`, JSON.stringify(deduped));
    const trimmed = root.trim().replace(/\/+$/, '');
    if (trimmed && trimmed !== canon) lsRemove(`${ROOTS_LOCAL_PREFIX}${trimmed}`);
    this.rootsRev = untrack(() => this.rootsRev) + 1;
    if (typeof window !== 'undefined') {
      window.dispatchEvent(new CustomEvent(IDE_PROJECTS_CHANGED_EVENT, { detail: { root: canon } }));
    }
  }
}

export const ideWorkspacePort = new IdeWorkspacePort();
