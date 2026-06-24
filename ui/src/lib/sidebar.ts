// Single source of truth for the left-sidebar module list, shared by the
// collapsed Rail, the expanded Navigator, and the Settings → Appearance
// customizer. Users can reorder and hide modules (persisted per-device in the
// `ui` store); these helpers are pure so they can be reasoned about (and tested
// via e2e) without Svelte/auth state — the caller supplies the RBAC predicate
// and the already-permitted plugin list.

import type { Feature } from './api/types';

/** A built-in module entry in the canonical registry. */
export interface SidebarModuleDef {
  /** Route id (`router.go(id)`) and the key used for ordering/hiding. */
  id: string;
  /** Icon name (see Icon.svelte). */
  icon: string;
  /** Display label. */
  label: string;
  /** RBAC feature gate (checked at 'view'). Omitted = ungated: always visible
   *  to any authenticated member (e.g. Vault, Message Brokers). */
  feature?: Feature;
  /** Agents/Connections render a nested live-session list in the Navigator and
   *  so need bespoke markup; every other module is a plain nav row. */
  special?: boolean;
}

/** A resolved, currently-available module (a built-in or a runtime plugin). */
export interface SidebarModule {
  id: string;
  icon: string;
  label: string;
  special?: boolean;
}

/**
 * Canonical default order — matches the shipped Navigator layout. Vault and
 * Message Brokers have no RBAC feature key (ungated). Runtime plugins are not
 * listed here; they are appended at resolve time (see {@link availableModules}).
 */
export const SIDEBAR_MODULES: SidebarModuleDef[] = [
  { id: 'agents', icon: 'terminal', label: 'Agents', feature: 'agents', special: true },
  { id: 'connections', icon: 'plug', label: 'Connections', feature: 'connections', special: true },
  { id: 'swarm', icon: 'grid', label: 'Swarm', feature: 'swarm' },
  { id: 'loops', icon: 'refresh', label: 'Goal Loops' },
  { id: 'git', icon: 'branch', label: 'Git', feature: 'git' },
  { id: 'product', icon: 'note', label: 'Product', feature: 'product' },
  { id: 'vault', icon: 'globe', label: 'Vault' },
  { id: 'canvas', icon: 'shapes', label: 'Canvas' },
  { id: 'api', icon: 'send', label: 'API', feature: 'api_client' },
  { id: 'database', icon: 'db', label: 'Database', feature: 'database' },
  { id: 'brokers', icon: 'box', label: 'Message Brokers' },
  { id: 'workflows', icon: 'split', label: 'Workflows', feature: 'workflows' },
  { id: 'skills-eval', icon: 'zap', label: 'Skills Evaluator', feature: 'skill_eval' },
  { id: 'insights', icon: 'gauge', label: 'Insights', feature: 'insights' },
  { id: 'usage', icon: 'chart', label: 'Usage', feature: 'usage' },
];

/**
 * The modules the current user may see: built-ins filtered by the RBAC `can`
 * predicate (ungated ones always pass), with the already-permitted runtime
 * plugins appended. Order here is the registry/plugin order — call
 * {@link resolveOrder} to apply the user's saved arrangement.
 */
export function availableModules(
  can: (feature: Feature) => boolean,
  plugins: SidebarModule[],
): SidebarModule[] {
  const builtins = SIDEBAR_MODULES.filter((m) => m.feature == null || can(m.feature)).map(
    (m): SidebarModule => ({ id: m.id, icon: m.icon, label: m.label, special: m.special }),
  );
  return [...builtins, ...plugins];
}

/**
 * Order `available` by the user's saved id order. Any available id NOT present
 * in `savedOrder` (a newly-shipped module, a freshly-installed plugin, or a
 * just-granted feature) keeps its natural order and is appended at the end so
 * nothing silently disappears. Saved ids that are no longer available are
 * ignored. The result is the FULL resolved order (visible + hidden).
 */
export function resolveOrder(available: SidebarModule[], savedOrder: string[]): SidebarModule[] {
  const byId = new Map(available.map((m) => [m.id, m]));
  const ordered: SidebarModule[] = [];
  for (const id of savedOrder) {
    const m = byId.get(id);
    if (m) {
      ordered.push(m);
      byId.delete(id);
    }
  }
  // Remaining (not in savedOrder) keep their availability order.
  for (const m of available) if (byId.has(m.id)) ordered.push(m);
  return ordered;
}

/** The visible subset of a resolved order: everything not in `hidden`. */
export function visibleOrder(ordered: SidebarModule[], hidden: string[]): SidebarModule[] {
  const h = new Set(hidden);
  return ordered.filter((m) => !h.has(m.id));
}
