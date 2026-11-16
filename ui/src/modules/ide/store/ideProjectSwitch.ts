// The two moments of a project switch, called by `resetTargetLifecycleState`:
// `leaveIdeTarget` before the store is cleared (remember the tree, park a live run,
// let participants save) and `enterIdeTarget` after (record it in recents, restore
// the tree, let participants restore, re-adopt a parked run). See
// `ideProjectMemory.ts`, `ideParkedRuns.ts` and `../shell/ideProjectRecents.svelte.ts`.

import { ideProjectRecents, sameRootKey, setRecentsExclusion } from '../shell/ideProjectRecents.svelte';
import { ideProjectHealth } from './ideProjectHealth.svelte';
import { ideWindowTitle } from './ideWindowTitle.svelte';
import { adoptParkedRun, parkRun, resumeParkedRunWatch, setParkedRunCurrentKey, setParkedRunReopener, type RunHost } from './ideParkedRuns';
import { enterProject, expandedToRemember, leaveProject, recallDurable, rememberDurable } from './ideProjectMemory';
import { ideBasename, ideTargetKey, type IdeNamedProject } from './ideTypes';

/** How many remembered folders are re-listed on return (each is one tree request). */
const MAX_RESTORED_DIRS = 40;

export interface ProjectSwitchHost extends RunHost {
  readonly projectRoots: readonly { path: string }[];
  expandedDirs: Record<string, boolean>;
  loadTree(dirPath?: string): Promise<void>;
}

type WithNamed = { namedProjects?: readonly IdeNamedProject[] };

function namedFor(host: ProjectSwitchHost, root: string): IdeNamedProject | null {
  const k = sameRootKey(root);
  return (host as WithNamed).namedProjects?.find((p) => sameRootKey(p.primary_root) === k) ?? null;
}

/** The project's name for a toast: the named project, `host:folder` for SSH, else the folder. */
export function projectLabelFor(host: ProjectSwitchHost): string {
  const t = host.target;
  if (t.kind === 'ssh') return `${t.connection_name || 'SSH'}:${ideBasename(t.root)}`;
  return namedFor(host, t.root)?.name || ideBasename(t.root) || t.root;
}

/** Pure: the remembered folders that can be shown — each one's parent is a root or is
 *  itself kept (a folder under a collapsed parent is not worth a request). */
export function reachableExpanded(saved: readonly string[], roots: readonly string[]): string[] {
  const rootSet = new Set(roots);
  const kept = new Set<string>();
  const sorted = [...saved].sort((a, b) => a.split('/').length - b.split('/').length);
  for (const p of sorted) {
    const parent = p.slice(0, p.lastIndexOf('/')) || '/';
    if (rootSet.has(parent) || kept.has(parent)) kept.add(p);
  }
  return sorted.filter((p) => kept.has(p));
}

let lastHost: ProjectSwitchHost | null = null;

function saveTree(host: ProjectSwitchHost, key: string): void {
  rememberDurable(key, { expanded: expandedToRemember(host.expandedDirs, host.projectRoots.map((r) => r.path)) });
}

export function leaveIdeTarget(host: ProjectSwitchHost): void {
  const t = host.target;
  if (!t.root) return;
  const key = ideTargetKey(t);
  saveTree(host, key);
  parkRun(host, key, projectLabelFor(host), {
    kind: t.kind, root: t.root, connection_id: t.connection_id ?? null, connection_name: t.connection_name ?? null,
    workspace_id: t.workspace_id ?? null, project_id: namedFor(host, t.root)?.id ?? null,
  });
  leaveProject(key);
}

export function enterIdeTarget(host: ProjectSwitchHost): void {
  ideProjectHealth.clear();
  lastHost = host;
  const t = host.target;
  ideWindowTitle.project = t.root ? projectLabelFor(host) : '';
  if (!t.root) return;
  const key = ideTargetKey(t);
  ideProjectRecents.record(t);
  const roots = host.projectRoots.map((r) => r.path);
  const saved = recallDurable(key)?.expanded ?? [];
  const restore = reachableExpanded(saved, roots).slice(0, MAX_RESTORED_DIRS);
  if (restore.length > 0) {
    const next = { ...host.expandedDirs };
    for (const p of restore) next[p] = true;
    host.expandedDirs = next;
    for (const p of restore) void host.loadTree(p);
  }
  enterProject(key);
  void adoptParkedRun(host, key).finally(() => resumeParkedRunWatch());
}

// Scratches has its own switcher row: never listed as a recent folder (lazy: no import cycle).
void import('../shell/ideScratches.svelte').then(({ scratches }) => {
  setRecentsExclusion((root) => Boolean(scratches.root) && sameRootKey(root) === sameRootKey(scratches.root));
}).catch(() => {});
setParkedRunCurrentKey(() => (lastHost?.target.root ? ideTargetKey(lastHost.target) : ''));
setParkedRunReopener((t) => {
  void import('../ideStore.svelte').then(async ({ ide }): Promise<void> => {
    if (t.project_id && ide.namedProjects.some((p) => p.id === t.project_id)) await ide.activateProject(t.project_id);
    else if (t.kind === 'ssh' && t.connection_id) await ide.setSshTarget(t.connection_id, t.root, t.connection_name ?? null);
    else await ide.setLocalTarget(t.root, t.workspace_id ?? null);
  });
});

// A reload keeps the tree and a live run of the project on screen (sessionStorage).
if (typeof window !== 'undefined') {
  window.addEventListener('pagehide', () => {
    const host = lastHost;
    if (!host?.target.root) return;
    const key = ideTargetKey(host.target);
    saveTree(host, key);
    parkRun(host, key, projectLabelFor(host), {
      kind: host.target.kind, root: host.target.root, connection_id: host.target.connection_id ?? null,
      connection_name: host.target.connection_name ?? null, workspace_id: host.target.workspace_id ?? null,
      project_id: namedFor(host, host.target.root)?.id ?? null,
    });
  });
}
