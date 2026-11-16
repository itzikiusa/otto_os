// Every project you opened can be found again (review I3-05 / I3-07 / I3-06).
//
// - Recents: each local folder or SSH target the IDE opens is recorded here (MRU,
//   bounded), with what is needed to reopen it — kind, root, connection id/name,
//   workspace id. Never a secret: an SSH target is reopened through its saved
//   connection, by id.
// - The switcher's "Workspaces" list is registered workspaces + recent local folders
//   that are not registered (`ideSwitcherFolders`), so a folder opened with "Open local
//   folder…" or `?root=` is still listed after you leave it — and after a reload.
// - Same-name folders are told apart by the shortest distinguishing parent
//   ("api — team-a"), in the label data the popover and header already render.
// - ⌘K rows: "IDE: Switch project…" and one "IDE: Open project <name>" per known
//   project (named, workspace, recent folder, SSH).

import type { Command } from '../../../lib/commands.svelte';
import { lsGet, lsSet } from '../../../lib/storage';
import { normalizeMacPrivatePath } from '../../../lib/fileUri';
import type { IdeNamedProject, IdeTargetSpec } from '../store/ideTypes';

export const IDE_RECENTS_KEY = 'otto.ide.recentTargets';
export const MAX_RECENTS = 20;

export interface IdeRecentTarget {
  kind: 'local' | 'ssh';
  root: string;
  connection_id?: string | null;
  connection_name?: string | null;
  workspace_id?: string | null;
  lastOpenedAt: number;
}

/** One row of the switcher's folder list (the shape `ws.workspaces` rows have). */
export interface IdeSwitcherFolder {
  id: string;
  name: string;
  root_path: string;
}

/** Recent rows carry this id prefix; `realWorkspaceId` maps it back to "no workspace". */
export const RECENT_ID_PREFIX = 'ide-recent:';

/** The one path spelling used to compare project roots (I3-21): `/private/var` ≡ `/var`,
 *  no trailing slash. Reuses the IDE's normaliser. */
export function sameRootKey(p: string | null | undefined): string {
  if (!p) return '';
  return normalizeMacPrivatePath(p.trim()).replace(/\/+$/, '') || '/';
}

export function recentKey(t: Pick<IdeRecentTarget, 'kind' | 'root' | 'connection_id'>): string {
  return t.kind === 'ssh' ? `ssh:${t.connection_id ?? ''}:${t.root}` : `local:${sameRootKey(t.root)}`;
}

function parseRecents(raw: string | null): IdeRecentTarget[] {
  try {
    const parsed = JSON.parse(raw ?? '[]') as unknown;
    if (!Array.isArray(parsed)) return [];
    return parsed
      .filter((r): r is IdeRecentTarget => Boolean(r) && typeof r === 'object' && (r.kind === 'local' || r.kind === 'ssh') && typeof r.root === 'string' && r.root.trim() !== '')
      .map((r) => ({
        kind: r.kind, root: r.root,
        connection_id: r.kind === 'ssh' ? (r.connection_id ?? null) : null,
        connection_name: r.kind === 'ssh' ? (r.connection_name ?? null) : null,
        workspace_id: r.kind === 'local' ? (r.workspace_id ?? null) : null,
        lastOpenedAt: Number(r.lastOpenedAt) || 0,
      }));
  } catch {
    return [];
  }
}

/** Roots that are never "recent" (the built-in Scratches folder has its own row). */
let excluded: (root: string) => boolean = () => false;
export function setRecentsExclusion(fn: (root: string) => boolean): void {
  excluded = fn;
}

/** Pure: the MRU list after opening `t`. Exported for tests. */
export function withRecent(list: readonly IdeRecentTarget[], t: IdeRecentTarget, max = MAX_RECENTS): IdeRecentTarget[] {
  const k = recentKey(t);
  return [t, ...list.filter((r) => recentKey(r) !== k)].slice(0, max);
}

class IdeProjectRecents {
  list = $state<IdeRecentTarget[]>(parseRecents(lsGet(IDE_RECENTS_KEY)));

  /** Record an opened target (the target switch calls this). SSH needs its connection id. */
  record(target: Pick<IdeTargetSpec, 'kind' | 'root' | 'connection_id' | 'connection_name' | 'workspace_id'>): void {
    const root = (target.root ?? '').trim().replace(/\/+$/, '') || (target.root ?? '').trim();
    if (!root || (root === '~' && target.kind === 'local')) return;
    if (target.kind === 'ssh' && !target.connection_id) return;
    if (target.kind !== 'ssh' && excluded(root)) return;
    const entry: IdeRecentTarget = {
      kind: target.kind === 'ssh' ? 'ssh' : 'local',
      root,
      connection_id: target.kind === 'ssh' ? target.connection_id ?? null : null,
      connection_name: target.kind === 'ssh' ? target.connection_name ?? null : null,
      workspace_id: target.kind === 'local' ? target.workspace_id ?? null : null,
      lastOpenedAt: Date.now(),
    };
    this.list = withRecent(this.list, entry);
    this.persist();
  }

  remove(t: Pick<IdeRecentTarget, 'kind' | 'root' | 'connection_id'>): void {
    const k = recentKey(t);
    this.list = this.list.filter((r) => recentKey(r) !== k);
    this.persist();
  }

  has(t: Pick<IdeRecentTarget, 'kind' | 'root' | 'connection_id'>): boolean {
    const k = recentKey(t);
    return this.list.some((r) => recentKey(r) === k);
  }

  private persist(): void {
    try { lsSet(IDE_RECENTS_KEY, JSON.stringify(this.list)); } catch { /* storage blocked */ }
  }
}

export const ideProjectRecents = new IdeProjectRecents();

if (typeof window !== 'undefined') {
  window.addEventListener('otto:ide-storage-cleared', () => { ideProjectRecents.list = []; });
}

// ── labels ──────────────────────────────────────────────────────────────────

function segments(p: string): string[] {
  return sameRootKey(p).split('/').filter(Boolean);
}

/** For rows whose names collide (case-insensitive), append the shortest parent path that
 *  tells them apart: "api — team-a", or "api — a/team" when one parent is not enough.
 *  Rows with a unique name keep it. Pure. */
export function disambiguateLabels<T extends { name: string; path: string }>(rows: readonly T[]): string[] {
  const out = rows.map((r) => r.name);
  const groups = new Map<string, number[]>();
  rows.forEach((r, i) => {
    const k = r.name.trim().toLowerCase();
    groups.set(k, [...(groups.get(k) ?? []), i]);
  });
  for (const idx of groups.values()) {
    const distinctPaths = new Set(idx.map((i) => sameRootKey(rows[i].path)));
    if (idx.length < 2 || distinctPaths.size < 2) continue;
    const parents = idx.map((i) => segments(rows[i].path).slice(0, -1).reverse());
    const maxDepth = Math.max(...parents.map((p) => p.length));
    let depth = 1;
    for (; depth <= maxDepth; depth++) {
      const tails = parents.map((p) => p.slice(0, depth).join('/'));
      if (new Set(tails).size === tails.length) break;
    }
    idx.forEach((i, j) => {
      const tail = parents[j].slice(0, Math.min(depth, parents[j].length)).reverse().join('/');
      if (tail) out[i] = `${rows[i].name} — ${tail}`;
    });
  }
  return out;
}

function basename(p: string): string {
  const s = segments(p);
  return s[s.length - 1] ?? p;
}

/** The switcher's folder rows: registered workspaces, then recent local folders that are
 *  not registered; names made unique. `workspaces` is `ws.workspaces` minus Scratches. */
export function ideSwitcherFolders(
  workspaces: readonly { id: string; name?: string | null; root_path?: string | null }[],
  recents: readonly IdeRecentTarget[] = ideProjectRecents.list,
): IdeSwitcherFolder[] {
  const rows: IdeSwitcherFolder[] = [];
  const seen = new Set<string>();
  for (const w of workspaces) {
    if (!w.root_path) continue;
    const k = sameRootKey(w.root_path);
    if (seen.has(k)) continue;
    seen.add(k);
    rows.push({ id: w.id, name: w.name || basename(w.root_path), root_path: w.root_path });
  }
  for (const r of recents) {
    if (r.kind !== 'local' || excluded(r.root)) continue;
    const k = sameRootKey(r.root);
    if (seen.has(k)) continue;
    seen.add(k);
    rows.push({ id: `${RECENT_ID_PREFIX}${r.root}`, name: basename(r.root), root_path: r.root });
  }
  const labels = disambiguateLabels(rows.map((r) => ({ name: r.name, path: r.root_path })));
  return rows.map((r, i) => (labels[i] === r.name ? r : { ...r, name: labels[i] }));
}

/** A recent row has no shell workspace behind it. */
export function realWorkspaceId(id: string | null | undefined): string | null {
  return !id || id.startsWith(RECENT_ID_PREFIX) ? null : id;
}

/** The header's name for a local folder target: its (disambiguated) row label. */
export function ideFolderLabel(
  root: string,
  workspaces: readonly { id: string; name?: string | null; root_path?: string | null }[],
): string {
  const k = sameRootKey(root);
  const row = ideSwitcherFolders(workspaces).find((r) => sameRootKey(r.root_path) === k);
  return row?.name || basename(root) || root;
}

// ── keyboard: ⌘K rows ───────────────────────────────────────────────────────

/** Open the header's project switcher the way its button does (it owns its open state). */
export function openIdeProjectSwitcher(): boolean {
  if (typeof document === 'undefined') return false;
  const btn = document.querySelector<HTMLButtonElement>('[data-testid="ide-target-switcher"]');
  if (!btn) return false;
  if (btn.getAttribute('aria-expanded') !== 'true') btn.click();
  return true;
}

export interface IdeProjectOpener {
  activateProject(id: string): Promise<unknown>;
  setLocalTarget(root: string, workspaceId?: string | null): Promise<unknown>;
  setSshTarget(connectionId: string, root: string, connectionName?: string | null): Promise<unknown>;
}

export interface IdeKnownProjectRow {
  key: string;
  title: string;
  keywords: string;
  open: (o: IdeProjectOpener) => void;
}

/** Every project you can reopen, once: named projects, workspace/recent folders, SSH. */
export function ideKnownProjects(
  named: readonly IdeNamedProject[],
  workspaces: readonly { id: string; name?: string | null; root_path?: string | null }[],
  recents: readonly IdeRecentTarget[] = ideProjectRecents.list,
): IdeKnownProjectRow[] {
  const out: IdeKnownProjectRow[] = [];
  for (const p of named) {
    out.push({ key: `named:${p.id}`, title: p.name, keywords: `${p.primary_root} ${p.folders.join(' ')}`, open: (o) => void o.activateProject(p.id) });
  }
  for (const f of ideSwitcherFolders(workspaces, recents)) {
    const wid = realWorkspaceId(f.id);
    out.push({ key: `local:${sameRootKey(f.root_path)}`, title: f.name, keywords: f.root_path, open: (o) => void o.setLocalTarget(f.root_path, wid) });
  }
  for (const r of recents) {
    if (r.kind !== 'ssh' || !r.connection_id) continue;
    const host = r.connection_name || 'SSH';
    const cid = r.connection_id;
    out.push({ key: recentKey(r), title: `${host}:${basename(r.root)}`, keywords: `ssh remote ${host} ${r.root}`, open: (o) => void o.setSshTarget(cid, r.root, r.connection_name ?? null) });
  }
  return out;
}

/** "IDE: Open project <name>" palette rows. */
export function ideProjectPaletteCommands(rows: readonly IdeKnownProjectRow[], opener: () => Promise<IdeProjectOpener>): Command[] {
  return rows.map((r) => ({
    id: `ide.openProject:${r.key}`,
    title: `IDE: Open project ${r.title}`,
    group: 'IDE',
    keywords: `switch project recent ${r.keywords}`,
    run: () => void opener().then((o) => r.open(o)),
  }));
}
