// What Run / Debug / ⌃R / ⌃D launch. The default is the file the person is
// looking at ("Current file"); a saved or auto-detected configuration wins only
// once the person picks it, and that pick is remembered per project. When the
// active tab can't run (a README), Run falls back to the configuration the
// person last launched — never silently to an arbitrary first one.

import { lsGet, lsSet } from '../../../lib/storage';
import { defaultRunnerForLanguage, ideTargetKey, type IdeOpenTab, type IdeRunConfig, type IdeTargetSpec } from './ideTypes';

/** The pinned first row of the launch popover. */
export const CURRENT_FILE_CONFIG_ID = 'current-file';

const KEY_PREFIX = 'otto.ide.launchTarget:';

export interface IdeLaunchMemory {
  /** The row the person picked (`current-file` or a config id). */
  selected: string | null;
  /** The last configuration (never `current-file`) the person launched. */
  lastUsed: string | null;
}

type TargetKeyed = Pick<IdeTargetSpec, 'kind' | 'root' | 'connection_id'>;
type TabLike = Pick<IdeOpenTab, 'name' | 'language'> | null | undefined;

export function loadLaunchMemory(target: TargetKeyed): IdeLaunchMemory {
  if (!target.root) return { selected: null, lastUsed: null };
  try {
    const raw = JSON.parse(lsGet(KEY_PREFIX + ideTargetKey(target)) ?? 'null') as Partial<IdeLaunchMemory> | null;
    return {
      selected: typeof raw?.selected === 'string' ? raw.selected : null,
      lastUsed: typeof raw?.lastUsed === 'string' ? raw.lastUsed : null,
    };
  } catch {
    return { selected: null, lastUsed: null };
  }
}

export function saveLaunchMemory(target: TargetKeyed, patch: Partial<IdeLaunchMemory>): void {
  if (!target.root) return;
  const next = { ...loadLaunchMemory(target), ...patch };
  lsSet(KEY_PREFIX + ideTargetKey(target), JSON.stringify(next));
}

/** Whether the active tab can be run/debugged as "Current file", and why not. */
export function currentFileRunnable(tab: TabLike): { ok: boolean; reason: string | null } {
  if (!tab) return { ok: false, reason: 'Open a file first' };
  if (defaultRunnerForLanguage(tab.language) === 'custom') return { ok: false, reason: 'Can’t run this file type' };
  return { ok: true, reason: null };
}

/** The selection after configs (re)load: a saved config already selected in
 *  this session (the configurations editor), else the remembered pick while it
 *  still exists, else "Current file". Never the first config by position. */
export function resolveLaunchSelection(
  memory: IdeLaunchMemory,
  configs: readonly Pick<IdeRunConfig, 'id' | 'auto_detected'>[],
  current: string | null = null,
): string {
  if (current === CURRENT_FILE_CONFIG_ID) return current;
  if (current && configs.some((c) => c.id === current && !c.auto_detected)) return current;
  const picked = memory.selected;
  if (picked && (picked === CURRENT_FILE_CONFIG_ID || configs.some((c) => c.id === picked))) return picked;
  return CURRENT_FILE_CONFIG_ID;
}

export type LaunchPlan =
  | { kind: 'config'; configId: string }
  | { kind: 'current-file' }
  | { kind: 'none'; reason: string };

/** What a toolbar/shortcut launch (no explicit config) should do right now. */
export function planLaunch(
  selectedId: string | null,
  tab: TabLike,
  memory: IdeLaunchMemory,
  configs: readonly Pick<IdeRunConfig, 'id'>[],
): LaunchPlan {
  if (selectedId && selectedId !== CURRENT_FILE_CONFIG_ID && configs.some((c) => c.id === selectedId)) {
    return { kind: 'config', configId: selectedId };
  }
  const runnable = currentFileRunnable(tab);
  if (runnable.ok) return { kind: 'current-file' };
  const last = memory.lastUsed;
  if (last && last !== CURRENT_FILE_CONFIG_ID && configs.some((c) => c.id === last)) return { kind: 'config', configId: last };
  const what = tab ? `${tab.name} can’t be run or debugged.` : 'No file is open.';
  return { kind: 'none', reason: `${what} Open a source file or pick a configuration.` };
}

/** "Current file (review.py)" for the toolbar selector. */
export function launchSelectorLabel(selected: Pick<IdeRunConfig, 'name'> | null, tab: TabLike): string {
  if (selected) return selected.name;
  return tab ? `Current file (${tab.name})` : 'Current file';
}

const ECOSYSTEM_RUNNERS: Record<string, readonly string[]> = {
  go: ['go_'],
  rust: ['cargo_'],
  java: ['java_', 'maven', 'gradle'],
  python: ['python', 'pytest'],
  javascript: ['node', 'npm', 'vitest'],
  typescript: ['node', 'npm', 'vitest'],
  tsx: ['node', 'npm', 'vitest'],
  jsx: ['node', 'npm', 'vitest'],
  shell: ['shell'],
};

/** Auto-detected configs with the active file's ecosystem first (stable otherwise). */
export function orderDetectedForLanguage<T extends Pick<IdeRunConfig, 'runner'>>(configs: readonly T[], language: string | null | undefined): T[] {
  const prefixes = (language && ECOSYSTEM_RUNNERS[language]) || [];
  if (prefixes.length === 0) return [...configs];
  const matches = (c: T): boolean => prefixes.some((p) => c.runner === p || c.runner.startsWith(p));
  return [...configs.filter(matches), ...configs.filter((c) => !matches(c))];
}
