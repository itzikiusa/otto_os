// What follows you back to a project (review I3-08 / I3-17): the tree's expanded
// folders, the agent thread and draft, the applied-proposal guards. One entry per
// project, keyed by `ideTargetKey`, least-recently-used out after MAX_PROJECTS.
//
// Two tiers on purpose:
// - `durable` (localStorage, survives a reload): folder paths and a thread id —
//   nothing a person typed and nothing from a file.
// - `volatile` (this page only): the composer draft and the applied-proposal Undo
//   maps. A draft can hold anything and Undo holds file contents; neither is written
//   to disk.
//
// Participants (the agent state, the tree) register save/restore once; the target
// switch (`resetTargetLifecycleState`) calls `leaveProject` before it clears the
// store and `enterProject` after, so no participant imports the store.

import { lsGet, lsSet } from '../../../lib/storage';

export const IDE_PROJECT_MEMORY_KEY = 'otto.ide.projectMemory';
export const MAX_PROJECTS = 12;
export const MAX_EXPANDED_DIRS = 200;

export interface IdeProjectDurableMemory {
  expanded?: string[];
  agentSessionId?: string | null;
  at: number;
}

export interface IdeProjectMemoryParticipant {
  /** Called with the project being left, before the store is reset. */
  save(key: string): void;
  /** Called with the project being entered, after the store is reset. */
  restore(key: string): void;
}

type DurableMap = Record<string, IdeProjectDurableMemory>;

function readDurable(): DurableMap {
  try {
    const parsed = JSON.parse(lsGet(IDE_PROJECT_MEMORY_KEY) ?? 'null') as unknown;
    return parsed && typeof parsed === 'object' && !Array.isArray(parsed) ? (parsed as DurableMap) : {};
  } catch {
    return {};
  }
}

function writeDurable(map: DurableMap): void {
  try {
    lsSet(IDE_PROJECT_MEMORY_KEY, JSON.stringify(map));
  } catch { /* quota or blocked storage: memory is a convenience */ }
}

/** Keep the MAX most recent entries (by `at`). Pure: exported for tests. */
export function pruneLru<T extends { at: number }>(map: Record<string, T>, max = MAX_PROJECTS): Record<string, T> {
  const keys = Object.keys(map);
  if (keys.length <= max) return map;
  const keep = keys.sort((a, b) => (map[b].at ?? 0) - (map[a].at ?? 0)).slice(0, max);
  const out: Record<string, T> = {};
  for (const k of keep) out[k] = map[k];
  return out;
}

let clock = 0;
/** Monotonic stamp: two writes in the same millisecond still order. */
function stamp(): number {
  clock = Math.max(clock + 1, Date.now());
  return clock;
}

export function rememberDurable(key: string, patch: Partial<Omit<IdeProjectDurableMemory, 'at'>>): void {
  if (!key) return;
  const map = readDurable();
  const next: IdeProjectDurableMemory = { ...(map[key] ?? {}), ...patch, at: stamp() };
  if (next.expanded) next.expanded = next.expanded.slice(0, MAX_EXPANDED_DIRS);
  map[key] = next;
  writeDurable(pruneLru(map));
}

export function recallDurable(key: string): IdeProjectDurableMemory | null {
  if (!key) return null;
  const entry = readDurable()[key];
  return entry && typeof entry === 'object' ? entry : null;
}

/** The memory key of a target — the same spelling as `ideTargetKey` (kept here so this
 *  leaf module imports nothing from the store). */
export function memoryKeyOf(t: { kind?: string; root?: string; connection_id?: string | null } | null | undefined): string {
  return t?.root ? `${t.kind}:${t.connection_id ?? ''}:${t.root}` : '';
}

// ── volatile tier ───────────────────────────────────────────────────────────

const volatile = new Map<string, { at: number; values: Record<string, unknown> }>();

export function stashVolatile(key: string, name: string, value: unknown): void {
  if (!key) return;
  const cur = volatile.get(key) ?? { at: 0, values: {} };
  cur.values[name] = value;
  cur.at = stamp();
  volatile.set(key, cur);
  if (volatile.size > MAX_PROJECTS) {
    const oldest = [...volatile.entries()].sort((a, b) => a[1].at - b[1].at)[0];
    if (oldest) volatile.delete(oldest[0]);
  }
}

export function recallVolatile<T>(key: string, name: string): T | undefined {
  return volatile.get(key)?.values[name] as T | undefined;
}

// ── participants ────────────────────────────────────────────────────────────

const participants = new Map<string, IdeProjectMemoryParticipant>();

/** Register (or replace) one participant by name. */
export function registerProjectMemoryParticipant(name: string, p: IdeProjectMemoryParticipant): void {
  participants.set(name, p);
}

export function leaveProject(key: string): void {
  if (!key) return;
  for (const p of participants.values()) {
    try { p.save(key); } catch { /* one participant never blocks a switch */ }
  }
}

export function enterProject(key: string): void {
  if (!key) return;
  for (const p of participants.values()) {
    try { p.restore(key); } catch { /* idem */ }
  }
}

/** Test seam. */
export function resetProjectMemoryForTests(): void {
  volatile.clear();
  participants.clear();
  clock = 0;
}

// ── tree expansion (pure helpers; the slice wires them) ─────────────────────

/** The expanded folders worth keeping: open, inside one of the project's roots, not a
 *  root itself (roots get their own default), parents before children, capped. */
export function expandedToRemember(expandedDirs: Record<string, boolean>, roots: readonly string[]): string[] {
  const rootSet = new Set(roots);
  const inProject = (p: string): boolean => roots.some((r) => p === r || p.startsWith(r.endsWith('/') ? r : `${r}/`));
  return Object.keys(expandedDirs)
    .filter((p) => expandedDirs[p] && !rootSet.has(p) && inProject(p))
    .sort((a, b) => a.split('/').length - b.split('/').length || a.localeCompare(b))
    .slice(0, MAX_EXPANDED_DIRS);
}
