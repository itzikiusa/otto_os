// Reactive IDE Projects & live-editor context bridge for the Threads module.
// Reads `localStorage` keys (`otto.ide.namedProjects.v1`, `otto.ide.lastRoot`,
// `otto.ide.roots:local:*`) + `window.__ottoIdeLiveContext` and listens to
// `'otto:ide-projects-changed'`, `'otto:ide-context-changed'`, and `'storage'`
// events without importing `src/modules/ide/*`.

import { api } from '../../lib/api/client';
import type { Session } from '../../lib/api/types';
import { router } from '../../lib/router.svelte';
import { lsGet, lsSet } from '../../lib/storage';
import { git } from '../../lib/stores/git.svelte';
import { SCRATCH_WORKSPACE_ID, ws } from '../../lib/stores/workspace.svelte';
import {
  buildIdePathAliases,
  canonicalIdePath,
  discoverIdeProjects,
  IDE_CONTEXT_CHANGED_EVENT,
  IDE_LAST_ROOT_KEY,
  IDE_NAMED_PROJECTS_KEY,
  IDE_OPEN_HANDOFF_KEY,
  IDE_OPEN_THREAD_EVENT,
  IDE_PROJECTS_CHANGED_EVENT,
  IDE_ROOTS_KEY_PREFIX,
  isSameOrSubpath,
  parseNamedIdeProjects,
  parseSavedAdditionalRoots,
  trimTrailingSlashes,
  type IdeLiveContextSnapshot,
  type IdeSidebarProject,
  type NamedIdeProjectRecord,
} from './ideProjectModel';

declare global {
  interface Window {
    __ottoIdeLiveContext?: IdeLiveContextSnapshot | null;
  }
}

function readAdditionalRootsFromStorage(): Record<string, string[]> {
  if (typeof window === 'undefined') return {};
  const map: Record<string, string[]> = {};
  try {
    for (let i = 0; i < window.localStorage.length; i++) {
      const key = window.localStorage.key(i);
      if (!key || !key.startsWith(IDE_ROOTS_KEY_PREFIX)) continue;
      const primary = trimTrailingSlashes(key.slice(IDE_ROOTS_KEY_PREFIX.length));
      if (!primary || primary === '~') continue;
      const roots = parseSavedAdditionalRoots(window.localStorage.getItem(key));
      map[primary] = roots;
    }
  } catch {
    // Ignore storage iteration failures in restricted contexts.
  }
  return map;
}

class IdeProjectsBridge {
  private rev = $state(0);
  private serverRootsByPrimary: Record<string, string[]> = {};
  private hydratedPrimaries = new Set<string>();

  constructor() {
    if (typeof window !== 'undefined') {
      const bump = (): void => {
        this.rev += 1;
      };
      window.addEventListener(IDE_PROJECTS_CHANGED_EVENT, bump);
      window.addEventListener(IDE_CONTEXT_CHANGED_EVENT, bump);
      window.addEventListener('storage', bump);
    }
  }

  refresh(): void {
    this.hydratedPrimaries.clear();
    this.rev += 1;
  }

  private hydrateServerEnv(primaryRoot: string): void {
    const clean = trimTrailingSlashes(primaryRoot);
    if (!clean || clean === '~' || this.hydratedPrimaries.has(clean)) return;
    if (lsGet('otto_mock') === '1') return;
    this.hydratedPrimaries.add(clean);
    void api
      .get<{ additional_roots?: string[] }>(
        `/ide/environments?kind=local&root=${encodeURIComponent(clean)}`,
      )
      .then((res) => {
        const extra = Array.isArray(res?.additional_roots)
          ? res.additional_roots.map(trimTrailingSlashes).filter((r) => Boolean(r && r !== '~'))
          : [];
        if (extra.length > 0) {
          this.serverRootsByPrimary[clean] = extra;
          this.rev += 1;
        }
      })
      .catch(() => {
        // Endpoint optional in test/offline harnesses; localStorage + live context remain authoritative.
      });
  }

  get liveContext(): IdeLiveContextSnapshot | null {
    void this.rev;
    if (typeof window === 'undefined') return null;
    return window.__ottoIdeLiveContext ?? null;
  }

  get namedProjects(): NamedIdeProjectRecord[] {
    void this.rev;
    return parseNamedIdeProjects(lsGet(IDE_NAMED_PROJECTS_KEY));
  }

  get lastRoot(): string | null {
    void this.rev;
    const raw = trimTrailingSlashes(lsGet(IDE_LAST_ROOT_KEY) ?? '');
    return raw && raw !== '~' ? raw : null;
  }

  projectsFor(sessions: readonly Session[] = []): IdeSidebarProject[] {
    void this.rev;
    const mergedAdditional: Record<string, string[]> = {
      ...readAdditionalRootsFromStorage(),
    };
    for (const [primary, extra] of Object.entries(this.serverRootsByPrimary)) {
      const prev = mergedAdditional[primary] ?? [];
      mergedAdditional[primary] = [...new Set([...prev, ...extra])];
    }
    const list = discoverIdeProjects({
      namedProjects: this.namedProjects,
      lastRoot: this.lastRoot,
      additionalRootsByPrimary: mergedAdditional,
      liveContext: this.liveContext,
      sessions,
    });
    for (let i = 0; i < list.length; i++) this.hydrateServerEnv(list[i].primaryRoot);
    return list;
  }

  /** Resolve search roots + `@alias/...` map + open IDE tabs for `@` completion. */
  collectMentionWorkspace(cwd?: string | null): {
    effectiveCwd: string;
    roots: string[];
    aliases: Record<string, string>;
    openTabs: Array<{ path: string; name: string; active: boolean }>;
  } {
    const projects = this.projectsFor();
    const live = this.liveContext;
    const cleanCwd = trimTrailingSlashes(cwd ?? '');
    const validCwd = cleanCwd && cleanCwd !== '~' ? cleanCwd : '';

    const roots: string[] = [];
    const pushRoot = (r?: string | null): void => {
      const c = trimTrailingSlashes(r ?? '');
      if (!c || c === '~') return;
      const canon = canonicalIdePath(c);
      if (!roots.some((x) => canonicalIdePath(x) === canon)) roots.push(c);
    };

    if (validCwd) pushRoot(validCwd);

    // Include all attached folders of any IDE project containing `validCwd` (multi-repo IDE project).
    for (let i = 0; i < projects.length; i++) {
      const p = projects[i];
      if (!validCwd || p.folders.some((f) => isSameOrSubpath(validCwd, f))) {
        for (let j = 0; j < p.folders.length; j++) pushRoot(p.folders[j]);
      }
    }

    if (roots.length === 0) {
      if (live?.root) {
        pushRoot(live.root);
        for (let i = 0; i < live.additionalRoots.length; i++) pushRoot(live.additionalRoots[i]);
      }
      if (this.lastRoot) pushRoot(this.lastRoot);
      if (ws.current?.root_path && ws.current.id !== SCRATCH_WORKSPACE_ID) pushRoot(ws.current.root_path);
      for (let i = 0; i < git.repos.length; i++) pushRoot(git.repos[i].path);
      for (let i = 0; i < ws.workspaces.length; i++) {
        const w = ws.workspaces[i];
        if (w.id !== SCRATCH_WORKSPACE_ID && w.root_path) pushRoot(w.root_path);
      }
    }

    const aliases = buildIdePathAliases({
      projects,
      repos: git.repos,
      workspaces: ws.workspaces.filter((w) => w.id !== SCRATCH_WORKSPACE_ID),
    });

    const openTabs = (live?.openTabs ?? []).map((t) => ({
      path: t.path,
      name: t.name,
      active: live?.activePath === t.path,
    }));

    return {
      effectiveCwd: validCwd || roots[0] || '',
      roots,
      aliases,
      openTabs,
    };
  }
}

export const ideProjects = new IdeProjectsBridge();

/** Navigate to the IDE module scoped to `root`, optionally opening `sessionId` in `IdeAgentPanel`. */
export function openProjectInIde(
  root: string,
  sessionId: string | null = null,
  projectId: string | null = null,
): void {
  const cleanRoot = trimTrailingSlashes(root);
  if (!cleanRoot) {
    router.go('ide');
    return;
  }
  const named = parseNamedIdeProjects(lsGet(IDE_NAMED_PROJECTS_KEY));
  const matchedNamed =
    (projectId ? named.find((p) => p.id === projectId) : null) ??
    named.find((p) => p.folders.some((f) => isSameOrSubpath(cleanRoot, f)));
  lsSet(IDE_LAST_ROOT_KEY, matchedNamed?.primary_root ?? cleanRoot);
  if (sessionId !== null) lsSet('otto.ide.agentOpen', '1');
  lsSet(
    IDE_OPEN_HANDOFF_KEY,
    JSON.stringify({
      root: cleanRoot,
      projectId: matchedNamed?.id ?? null,
      sessionId,
      ts: Date.now(),
    }),
  );
  if (typeof window !== 'undefined') {
    window.dispatchEvent(
      new CustomEvent(IDE_OPEN_THREAD_EVENT, {
        detail: { root: cleanRoot, projectId: matchedNamed?.id ?? null, sessionId },
      }),
    );
  }
  const qs = new URLSearchParams();
  if (matchedNamed) qs.set('project', matchedNamed.id);
  qs.set('root', cleanRoot);
  qs.set('t', String(Date.now()));
  router.go(`ide?${qs.toString()}`);
}
