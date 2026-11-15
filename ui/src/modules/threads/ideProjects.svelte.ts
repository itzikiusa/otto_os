// Reactive IDE Projects & live-editor context bridge for the Threads module.
// Consumes the shared `ui/src/lib/ideWorkspacePort.svelte.ts` and
// `ui/src/lib/ideProjectsModel.ts` contracts without importing `src/modules/ide/*`.

import { untrack } from 'svelte';
import { api } from '../../lib/api/client';
import type { Session } from '../../lib/api/types';
import {
  fetchNamedIdeProjectsFromServer,
  loadNamedIdeProjectsLocal,
  LS_IDE_PROJECTS,
  normalizeNamedIdeProjects,
  normalizeProjectPath,
  type NamedIdeProject,
} from '../../lib/ideProjectsModel';
import { ideWorkspacePort } from '../../lib/ideWorkspacePort.svelte';
import { router } from '../../lib/router.svelte';
import { lsGet, lsSet } from '../../lib/storage';
import { git } from '../../lib/stores/git.svelte';
import { SCRATCH_WORKSPACE_ID, ws } from '../../lib/stores/workspace.svelte';
import {
  basenameOfPath,
  buildIdePathAliases,
  canonicalIdePath,
  discoverIdeProjects,
  IDE_OPEN_HANDOFF_KEY,
  IDE_OPEN_THREAD_EVENT,
  IDE_PROJECTS_CHANGED_EVENT,
  isSameOrSubpath,
  trimTrailingSlashes,
  type IdeLiveContextSnapshot,
  type IdeSidebarProject,
} from './ideProjectModel';

class IdeProjectsBridge {
  private rev = $state(0);
  private serverProjects = $state<NamedIdeProject[] | null>(null);
  private hydratedProjects = false;
  private serverRootsByPrimary: Record<string, string[]> = {};
  private hydratedPrimaries = new Set<string>();

  private forceNextFetch = false;

  constructor() {
    if (typeof window !== 'undefined') {
      const onProjectsChanged = (ev: Event): void => {
        const detail = (ev as CustomEvent<{ projects?: unknown }>).detail;
        if (Array.isArray(detail?.projects)) {
          this.serverProjects = normalizeNamedIdeProjects(
            detail.projects as ReadonlyArray<NamedIdeProject | Record<string, unknown>>,
          );
          this.hydratedProjects = true;
        }
        this.rev = untrack(() => this.rev) + 1;
      };
      const onStorage = (ev: StorageEvent): void => {
        if (ev.key && !ev.key.startsWith('otto.ide.')) return;
        if (!ev.key || ev.key === LS_IDE_PROJECTS) {
          this.serverProjects = loadNamedIdeProjectsLocal();
          this.hydratedProjects = true;
        }
        this.rev = untrack(() => this.rev) + 1;
      };
      window.addEventListener(IDE_PROJECTS_CHANGED_EVENT, onProjectsChanged);
      window.addEventListener('storage', onStorage);
    }
  }

  refresh(): void {
    this.hydratedPrimaries.clear();
    this.hydratedProjects = false;
    this.forceNextFetch = true;
    this.serverProjects = null;
    this.rev = untrack(() => this.rev) + 1;
  }

  private ensureBackendProjects(): void {
    if (this.hydratedProjects || lsGet('otto_mock') === '1') return;
    this.hydratedProjects = true;
    const force = this.forceNextFetch;
    this.forceNextFetch = false;
    void fetchNamedIdeProjectsFromServer({ force })
      .then((list) => {
        if (list) {
          const prevJson = this.serverProjects ? JSON.stringify(this.serverProjects) : null;
          this.serverProjects = list;
          if (prevJson !== JSON.stringify(list)) {
            this.rev = untrack(() => this.rev) + 1;
          }
        }
      })
      .catch(() => {
        // Offline/test harnesses fall back cleanly to `loadNamedIdeProjectsLocal()`.
      });
  }

  private hydrateServerEnv(primaryRoot: string): void {
    const clean = trimTrailingSlashes(primaryRoot);
    if (!clean || clean === '~' || this.hydratedPrimaries.has(clean)) return;
    if (lsGet('otto_mock') === '1') return;
    this.hydratedPrimaries.add(clean);
    void api
      .get<{ env?: { additional_roots?: string[] }; additional_roots?: string[] }>(
        `/ide/environments?kind=local&root=${encodeURIComponent(clean)}`,
      )
      .then((res) => {
        const rawRoots = res?.env?.additional_roots ?? res?.additional_roots;
        const extra = Array.isArray(rawRoots)
          ? rawRoots.map(trimTrailingSlashes).filter((r) => Boolean(r && r !== '~'))
          : [];
        if (extra.length > 0) {
          this.serverRootsByPrimary[clean] = extra;
          this.rev = untrack(() => this.rev) + 1;
        }
      })
      .catch(() => {
        // Endpoint optional in test/offline harnesses; local + live context remain authoritative.
      });
  }

  get liveContext(): IdeLiveContextSnapshot | null {
    void this.rev;
    const snap = ideWorkspacePort.current;
    if (!snap.root && snap.tabs.length === 0 && !snap.activePath) return null;
    return {
      root: snap.root ?? '',
      additionalRoots: snap.additionalRoots,
      activePath: snap.activePath,
      openTabs: snap.tabs.map((t) => ({
        path: t.path,
        name: basenameOfPath(t.path) || t.path,
      })),
    };
  }

  get namedProjects(): NamedIdeProject[] {
    void this.rev;
    this.ensureBackendProjects();
    return this.serverProjects ?? loadNamedIdeProjectsLocal();
  }

  get lastRoot(): string | null {
    void this.rev;
    const raw = normalizeProjectPath(ideWorkspacePort.lastRoot ?? '');
    return raw && raw !== '~' ? raw : null;
  }

  projectsFor(sessions: readonly Session[] = []): IdeSidebarProject[] {
    void this.rev;
    const named = this.namedProjects;
    const last = this.lastRoot;
    const live = this.liveContext;

    const candidatePrimaries = new Set<string>();
    for (const p of named) {
      if (p.primaryRoot) candidatePrimaries.add(trimTrailingSlashes(p.primaryRoot));
    }
    if (last) candidatePrimaries.add(last);
    if (live?.root) candidatePrimaries.add(trimTrailingSlashes(live.root));
    for (const s of sessions) {
      const metaRoot = typeof s.meta?.ide_root === 'string' ? s.meta.ide_root : null;
      if (metaRoot) candidatePrimaries.add(trimTrailingSlashes(metaRoot));
      else if (s.meta?.surface === 'ide' && s.cwd) candidatePrimaries.add(trimTrailingSlashes(s.cwd));
    }

    const mergedAdditional: Record<string, string[]> = {};
    for (const primary of candidatePrimaries) {
      if (!primary || primary === '~') continue;
      const localExtra = ideWorkspacePort.rootsForLocal(primary);
      const serverExtra = this.serverRootsByPrimary[primary] ?? [];
      if (localExtra.length > 0 || serverExtra.length > 0) {
        mergedAdditional[primary] = [...new Set([...localExtra, ...serverExtra])];
      }
    }
    for (const [primary, extra] of Object.entries(this.serverRootsByPrimary)) {
      const prev = mergedAdditional[primary] ?? [];
      mergedAdditional[primary] = [...new Set([...prev, ...extra])];
    }

    const list = discoverIdeProjects({
      namedProjects: named,
      lastRoot: last,
      additionalRootsByPrimary: mergedAdditional,
      liveContext: live,
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
  const named = ideProjects.namedProjects;
  const matchedNamed =
    (projectId ? named.find((p) => p.id === projectId) : null) ??
    named.find((p) => p.folders.some((f) => isSameOrSubpath(cleanRoot, f)));
  ideWorkspacePort.setLastRoot(matchedNamed?.primaryRoot ?? cleanRoot);
  if (sessionId !== null) ideWorkspacePort.setAgentDockOpen(true);
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
