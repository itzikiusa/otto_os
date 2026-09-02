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
   *  to any authenticated member (e.g. Goal Loops, Canvas). */
  feature?: Feature;
  /** Alternative gates: visible when ANY of these features is viewable (used by
   *  Connections, which fronts both the connections and database features). */
  featureAny?: Feature[];
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
  { id: 'run-with-otto', icon: 'play', label: 'Run with Otto', feature: 'run_with_otto' },
  { id: 'mission-control', icon: 'radar', label: 'Mission Control', feature: 'mission_control' },
  // The unified hub: SSH/custom terminals + databases + Kafka clusters live in
  // ONE tree here, so there are no separate Database / Message Brokers entries —
  // their views (`#/database`, `#/brokers`) are reached by opening a row and
  // highlight this entry (see navIdForModule). Visible with EITHER feature.
  // Plain row (no nested open-connections list in the sidebar): open
  // connections already show as tabs on the Agents view.
  {
    id: 'connections',
    icon: 'plug',
    label: 'Connections',
    featureAny: ['connections', 'database'],
  },
  { id: 'swarm', icon: 'grid', label: 'Swarm', feature: 'swarm' },
  { id: 'loops', icon: 'refresh', label: 'Goal Loops' },
  { id: 'proof', icon: 'check', label: 'Proof', feature: 'proof_pack' },
  { id: 'git', icon: 'branch', label: 'Git', feature: 'git' },
  { id: 'product', icon: 'note', label: 'Product', feature: 'product' },
  // Vault v3 — the file-backed docs home (Obsidian-parity markdown vaults, OKF).
  { id: 'vault', icon: 'globe', label: 'Vault' },
  { id: 'canvas', icon: 'shapes', label: 'Canvas' },
  { id: 'browser', label: 'Browser', icon: 'globe', feature: 'browser' },
  // AWS fronts seven keys (account mgmt + one per service) — visible when ANY
  // of them is viewable, same shape as Connections/Database.
  { id: 'aws', icon: 'cloud', label: 'AWS', featureAny: ['aws', 'aws_s3', 'aws_sqs', 'aws_ec2', 'aws_athena', 'aws_eks'] },
  { id: 'kubernetes', icon: 'helm', label: 'Kubernetes', feature: 'kubernetes' },
  { id: 'api', icon: 'send', label: 'API', feature: 'api_client' },
  { id: 'mcp', icon: 'plug', label: 'MCP Control Plane', feature: 'mcp' },
  { id: 'workflows', icon: 'split', label: 'Workflows', feature: 'workflows' },
  { id: 'scheduled-tasks', icon: 'clock', label: 'Scheduled Tasks', feature: 'scheduled_tasks' },
  // Personal Agents share the scheduled_tasks feature gate (same RBAC axis on
  // the daemon: View for GET, Edit for writes).
  { id: 'personal-agents', icon: 'user', label: 'Personal Agents', feature: 'scheduled_tasks' },
  { id: 'skills-eval', icon: 'zap', label: 'Skills Lab', feature: 'skill_eval' },
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
  const builtins = SIDEBAR_MODULES.filter((m) =>
    m.featureAny ? m.featureAny.some(can) : m.feature == null || can(m.feature),
  ).map((m): SidebarModule => ({ id: m.id, icon: m.icon, label: m.label, special: m.special }));
  return [...builtins, ...plugins];
}

/**
 * The sidebar entry a router module belongs to. The Database Explorer and
 * Message Brokers views have no nav entries of their own — they are opened from
 * the unified Connections hub, so their routes highlight `connections`.
 */
export function navIdForModule(routerModule: string): string {
  if (routerModule === 'database' || routerModule === 'brokers') return 'connections';
  return routerModule;
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
