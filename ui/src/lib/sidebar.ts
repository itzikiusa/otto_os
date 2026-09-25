// Single source of truth for the left-sidebar module list, shared by the
// collapsed Rail, the expanded Navigator, the phone BottomNav, the ⌘K "Go to"
// commands and the Settings → Appearance customizer. Users can reorder and hide
// modules, pin modules into a Favorites section and reorder the sections
// themselves (all persisted per-device in the `ui` store); these helpers are
// pure so they can be unit-tested (unit/sidebar.test.ts) without Svelte/auth
// state — the caller supplies the RBAC predicate and the already-permitted
// plugin list.

import type { IconName } from './components/Icon.svelte';
import type { Feature } from './api/types';

/**
 * Sidebar sections, macOS source-list style: every module belongs to exactly
 * one, and the Navigator renders them under collapsible headers (the Rail
 * draws a thin separator between them) — in THIS order by default, or in the
 * user's saved section order (see {@link resolveGroupOrder}). The user's saved
 * module order only ever rearranges modules WITHIN their section. Runtime
 * plugins get their own trailing section. A favorited module leaves its
 * section for the pinned-first Favorites section (see {@link sidebarSections}).
 */
export type SidebarGroupId = 'work' | 'automate' | 'build' | 'infra' | 'insight' | 'plugins';

export interface SidebarGroupDef {
  id: SidebarGroupId;
  label: string;
}

export const SIDEBAR_GROUPS: SidebarGroupDef[] = [
  { id: 'work', label: 'Work' },
  { id: 'automate', label: 'Automate' },
  { id: 'build', label: 'Build' },
  { id: 'infra', label: 'Infrastructure' },
  { id: 'insight', label: 'Insight' },
  { id: 'plugins', label: 'Plugins' },
];

/** A built-in module entry in the canonical registry. */
export interface SidebarModuleDef {
  /** Route id (`router.go(id)`) and the key used for ordering/hiding. */
  id: string;
  /** Icon name (see Icon.svelte). Unique across modules — the collapsed Rail
   *  shows icons only, so a shared glyph makes two entries indistinguishable. */
  icon: IconName;
  /** Display label. */
  label: string;
  /** Sidebar section (see {@link SIDEBAR_GROUPS}). */
  group: SidebarGroupId;
  /** Extra ⌘K fuzzy-match terms for the derived "Go to …" command. */
  keywords?: string;
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
  icon: IconName;
  label: string;
  group: SidebarGroupId;
  keywords?: string;
  special?: boolean;
}

/** A permitted runtime plugin as the callers build it (its section is implied). */
export type SidebarPluginEntry = Omit<SidebarModule, 'group'>;

/**
 * Canonical default order — section by section, so the shipped flat order and
 * the grouped layout agree. Vault and Message Brokers have no RBAC feature key
 * (ungated). Runtime plugins are not listed here; they are appended at resolve
 * time (see {@link availableModules}).
 */
export const SIDEBAR_MODULES: SidebarModuleDef[] = [
  // ── Work ──
  // Home — the personal dashboard (views of live boxes). Ungated: every box
  // kind is gated individually by its own feature, so the page itself is safe
  // for any role.
  { id: 'home', icon: 'home', label: 'Home', group: 'work', keywords: 'dashboard overview boxes views' },
  // Assistant — Otto's personal assistant: one front door that chats (rendered
  // from the CLI transcript), remembers, reminds, runs tasks and delegates to
  // Personal Agents, asking before anything leaves the Mac. `#/assistant`.
  { id: 'assistant', icon: 'assistant', label: 'Assistant', group: 'work', keywords: 'otto personal assistant chat ask remember remind reminder memory tasks approvals needs you' },
  { id: 'agents', icon: 'terminal', label: 'Agents', group: 'work', feature: 'agents', special: true, keywords: 'sessions terminal claude codex shell' },
  // History — every past Claude/Codex conversation (Otto sessions + transcripts
  // found on disk), read-only, resumable. Lives in the Agents group and shares
  // its RBAC gate; the route is `#/history` (NOT `#/agents/…`, whose second
  // segment is a session id). See docs/design/conversation-view.md §5.3.
  { id: 'history', icon: 'clock', label: 'History', group: 'work', feature: 'agents', keywords: 'conversations transcripts past resume' },
  { id: 'run-with-otto', icon: 'play', label: 'Run with Otto', group: 'work', feature: 'run_with_otto', keywords: 'one button launch jira github issue pr confluence finding test story channel review proof approval' },
  { id: 'mission-control', icon: 'radar', label: 'Mission Control', group: 'work', feature: 'mission_control', keywords: 'work graph overview approvals spend running waiting' },
  // ── Automate ──
  { id: 'swarm', icon: 'grid', label: 'Swarm', group: 'automate', feature: 'swarm', keywords: 'agents team org orchestrator kanban board company' },
  { id: 'loops', icon: 'refresh', label: 'Goal Loops', group: 'automate', keywords: 'goal loop iterate autonomous objective plan execute evaluate' },
  { id: 'workflows', icon: 'split', label: 'Workflows', group: 'automate', feature: 'workflows', keywords: 'pipeline steps runs pr review automation' },
  { id: 'scheduled-tasks', icon: 'calendar', label: 'Scheduled Tasks', group: 'automate', feature: 'scheduled_tasks', keywords: 'cron recurring job report cadence hourly daily' },
  // Personal Agents share the scheduled_tasks feature gate (same RBAC axis on
  // the daemon: View for GET, Edit for writes).
  { id: 'personal-agents', icon: 'user', label: 'Personal Agents', group: 'automate', feature: 'scheduled_tasks', keywords: 'persona soul bot room chat schedule recap' },
  // ── Build ──
  { id: 'git', icon: 'branch', label: 'Git', group: 'build', feature: 'git', keywords: 'repos prs pull requests diff commit' },
  { id: 'proof', icon: 'check', label: 'Proof', group: 'build', feature: 'proof_pack', keywords: 'proof pack evidence badge verified tests ci approval audit' },
  { id: 'product', icon: 'note', label: 'Product', group: 'build', feature: 'product', keywords: 'story jira confluence analysis rfc' },
  // Vault v3 — the file-backed docs home (Obsidian-parity markdown vaults, OKF).
  { id: 'vault', icon: 'book', label: 'Vault', group: 'build', keywords: 'docs notes markdown obsidian okf knowledge graph' },
  // Design Hall — one library for every design studio (Frames, Graphics, Site,
  // 3D, Whiteboard, Brand Kit, Spatial). It replaces the Canvas entry: Canvas
  // lives on as the Whiteboard studio (`#/canvas` still routes; see navIdForModule).
  { id: 'design', icon: 'designHall', label: 'Design Hall', group: 'build', feature: 'design', keywords: 'design studio frames graphics site 3d whiteboard canvas brand kit mockup diagram sketch excalidraw mermaid d2 spatial' },
  { id: 'skills-eval', icon: 'zap', label: 'Skills Lab', group: 'build', feature: 'skill_eval', keywords: 'skill lab evaluate validate review edit improve' },
  // ── Infrastructure ──
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
    group: 'infra',
    featureAny: ['connections', 'database'],
    keywords: 'ssh mysql postgres redis mongodb clickhouse database kafka',
  },
  // AWS fronts seven keys (account mgmt + one per service) — visible when ANY
  // of them is viewable, same shape as Connections/Database.
  { id: 'aws', icon: 'cloud', label: 'AWS', group: 'infra', featureAny: ['aws', 'aws_s3', 'aws_sqs', 'aws_ec2', 'aws_athena', 'aws_eks', 'aws_rds'], keywords: 'amazon cloud s3 bucket sqs queue ec2 instance athena query eks account profile sso' },
  { id: 'kubernetes', icon: 'helm', label: 'Kubernetes', group: 'infra', feature: 'kubernetes', keywords: 'k8s kubectl k9s cluster context namespace pod deployment logs exec rollout argo argocd restart' },
  { id: 'api', icon: 'send', label: 'API', group: 'infra', feature: 'api_client', keywords: 'api client postman http request rest curl' },
  { id: 'browser', icon: 'compass', label: 'Browser', group: 'infra', feature: 'browser', keywords: 'reader mode tabs annotate url fetch page web' },
  { id: 'mcp', icon: 'server', label: 'MCP Control Plane', group: 'infra', feature: 'mcp', keywords: 'model context protocol server tool governance allowlist policy approval audit injection risk' },
  // ── Insight ──
  { id: 'insights', icon: 'gauge', label: 'Insights', group: 'insight', feature: 'insights', keywords: 'reports daily weekly monthly summary analytics activity' },
  { id: 'usage', icon: 'chart', label: 'Usage', group: 'insight', feature: 'usage', keywords: 'cost tokens clickhouse metrics cpu ram billing analytics' },
];

/**
 * The modules the current user may see: built-ins filtered by the RBAC `can`
 * predicate (ungated ones always pass), with the already-permitted runtime
 * plugins appended (in the Plugins section). Order here is the registry/plugin
 * order — call {@link resolveOrder} to apply the user's saved arrangement.
 */
export function availableModules(
  can: (feature: Feature) => boolean,
  plugins: SidebarPluginEntry[],
): SidebarModule[] {
  const builtins = SIDEBAR_MODULES.filter((m) =>
    m.featureAny ? m.featureAny.some(can) : m.feature == null || can(m.feature),
  ).map(
    (m): SidebarModule => ({
      id: m.id,
      icon: m.icon,
      label: m.label,
      group: m.group,
      keywords: m.keywords,
      special: m.special,
    }),
  );
  return [...builtins, ...plugins.map((p): SidebarModule => ({ ...p, group: 'plugins' }))];
}

/**
 * The sidebar entry a router module belongs to. The Database Explorer and
 * Message Brokers views have no nav entries of their own — they are opened from
 * the unified Connections hub, so their routes highlight `connections`.
 */
export function navIdForModule(routerModule: string): string {
  if (routerModule === 'database' || routerModule === 'brokers') return 'connections';
  // Canvas is Design Hall's Whiteboard studio.
  if (routerModule === 'canvas') return 'design';
  return routerModule;
}

/**
 * The sidebar id a route highlights (`router.parts`). Plugin entries carry a
 * `plugin/<slug>` id while the route module is just `plugin`; Agents also owns
 * the default ('') route; database/brokers highlight Connections.
 */
export function activeNavId(parts: string[]): string {
  const mod = parts[0] ?? '';
  if (mod === 'plugin') return `plugin/${parts[1] ?? ''}`;
  if (mod === '') return 'agents';
  return navIdForModule(mod);
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

/** The Favorites section's id. Not a module group — any module can be
 *  favorited — so it is never a {@link SidebarModule.group}. */
export const FAVORITES_ID = 'favorites';

/** A rendered section: one of {@link SIDEBAR_GROUPS} or Favorites. */
export type SidebarSectionId = SidebarGroupId | typeof FAVORITES_ID;

export interface SidebarSectionDef {
  id: SidebarSectionId;
  label: string;
}

/** Favorites: always the FIRST section, and only rendered while it holds at
 *  least one module the user can see. */
export const FAVORITES_SECTION: SidebarSectionDef = { id: FAVORITES_ID, label: 'Favorites' };

/** One rendered sidebar section: its definition + its modules in display order. */
export interface SidebarSection {
  group: SidebarSectionDef;
  modules: SidebarModule[];
}

/**
 * The section order: the user's saved group ids first (duplicates, unknown ids
 * and `favorites` — which is always first — are ignored), then every group the
 * saved list doesn't mention in the shipped {@link SIDEBAR_GROUPS} order, so a
 * newly-shipped section is appended and never lost.
 */
export function resolveGroupOrder(saved: readonly string[]): SidebarGroupDef[] {
  const byId = new Map(SIDEBAR_GROUPS.map((g) => [g.id as string, g]));
  const out: SidebarGroupDef[] = [];
  for (const id of saved) {
    const g = byId.get(id);
    if (g) {
      out.push(g);
      byId.delete(id);
    }
  }
  for (const g of SIDEBAR_GROUPS) if (byId.has(g.id)) out.push(g);
  return out;
}

/**
 * Split an ordered module list into sections — in {@link SIDEBAR_GROUPS} order,
 * or the user's `groupOrder` (resolved by {@link resolveGroupOrder}) — keeping
 * each module's relative (saved) order inside its section. Empty sections are
 * dropped — so a section whose modules are all hidden disappears.
 */
export function groupModules(list: SidebarModule[], groupOrder: readonly string[] = []): SidebarSection[] {
  return resolveGroupOrder(groupOrder)
    .map((group) => ({
      group,
      modules: list.filter((m) => m.group === group.id),
    }))
    .filter((s) => s.modules.length > 0);
}

/**
 * The favorited modules of `list`, in the user's `favorites` order. `list` is
 * already RBAC/feature-filtered, so a favorite the user can't see (a revoked
 * feature, an uninstalled plugin, an unknown id) is simply skipped — its id
 * stays saved, and it returns to its slot if it becomes available again.
 */
export function favoriteModules(list: SidebarModule[], favorites: readonly string[]): SidebarModule[] {
  const byId = new Map(list.map((m) => [m.id, m]));
  const out: SidebarModule[] = [];
  for (const id of favorites) {
    const m = byId.get(id);
    if (m) {
      out.push(m);
      byId.delete(id); // a duplicated saved id renders once
    }
  }
  return out;
}

/**
 * The sections the sidebar renders: Favorites first (only when it holds at
 * least one module of `list`), then every group in the user's section order.
 * A favorited module appears ONLY in Favorites — it leaves its own group, and
 * returns to its saved slot there when unfavorited.
 */
export function sidebarSections(
  list: SidebarModule[],
  favorites: readonly string[],
  groupOrder: readonly string[] = [],
): SidebarSection[] {
  const favs = favoriteModules(list, favorites);
  const favIds = new Set(favs.map((m) => m.id));
  const rest = groupModules(
    list.filter((m) => !favIds.has(m.id)),
    groupOrder,
  );
  return favs.length > 0 ? [{ group: FAVORITES_SECTION, modules: favs }, ...rest] : rest;
}

/**
 * `ids` with `id` swapped one slot up (`delta` -1) or down (+1) with its
 * nearest neighbour that `present` accepts — so a move is always visible even
 * when the saved list holds ids that aren't rendered (a hidden-by-RBAC
 * favorite, an empty section). Returns null when there is no such neighbour
 * (already first / last) or `id` isn't in `ids`.
 */
export function moveAmong(
  ids: readonly string[],
  id: string,
  delta: -1 | 1,
  present: (id: string) => boolean = () => true,
): string[] | null {
  const i = ids.indexOf(id);
  if (i < 0) return null;
  let j = i + delta;
  while (j >= 0 && j < ids.length && !present(ids[j])) j += delta;
  if (j < 0 || j >= ids.length) return null;
  const next = [...ids];
  [next[i], next[j]] = [next[j], next[i]];
  return next;
}

/**
 * Drag-reorder: `ids` with `from` pulled out and reinserted at `to`'s slot (so
 * it lands before `to` when dragged up, after it when dragged down). Returns
 * null when either id is missing or they're the same.
 */
export function reorderAmong(ids: readonly string[], from: string, to: string): string[] | null {
  if (from === to) return null;
  const i = ids.indexOf(from);
  const j = ids.indexOf(to);
  if (i < 0 || j < 0) return null;
  const next = [...ids];
  next.splice(i, 1);
  next.splice(j, 0, from);
  return next;
}

/**
 * `favorites` with `id` added — before `beforeId` when given and present
 * (dropping a row onto a favorite), else at the end. An id already there is
 * moved, never duplicated.
 */
export function insertFavorite(favorites: readonly string[], id: string, beforeId?: string | null): string[] {
  const next = favorites.filter((x) => x !== id);
  const at = beforeId ? next.indexOf(beforeId) : -1;
  if (at < 0) next.push(id);
  else next.splice(at, 0, id);
  return next;
}

/**
 * The full resolved order with `id` moved one slot up (`delta` -1) or down (+1)
 * WITHIN its section: it swaps with its nearest same-section neighbour, so every
 * move is visible even when an old saved flat order interleaves sections.
 * Returns null when there is no neighbour that way (already first / last).
 */
export function moveWithinGroup(ordered: SidebarModule[], id: string, delta: -1 | 1): string[] | null {
  const ids = ordered.map((m) => m.id);
  const i = ids.indexOf(id);
  if (i < 0) return null;
  const group = ordered[i].group;
  let j = i + delta;
  while (j >= 0 && j < ordered.length && ordered[j].group !== group) j += delta;
  if (j < 0 || j >= ordered.length) return null;
  [ids[i], ids[j]] = [ids[j], ids[i]];
  return ids;
}

/** Display name of a section. */
export function groupLabel(group: SidebarGroupId): string {
  return SIDEBAR_GROUPS.find((g) => g.id === group)?.label ?? '';
}

/**
 * Display label for a router module id (the phone top bar). Registry label when
 * the route has a sidebar entry, else a known name for the routes that don't
 * (Database / Message Brokers live under Connections), else the id with its
 * first letter capitalised — never the raw kebab-case id.
 */
export function moduleLabel(routerModule: string): string {
  const def = SIDEBAR_MODULES.find((m) => m.id === routerModule);
  if (def) return def.label;
  const other: Record<string, string> = {
    database: 'Database',
    brokers: 'Message Brokers',
    settings: 'Settings',
    walkthroughs: 'Help',
    plugin: 'Plugin',
  };
  return other[routerModule] ?? routerModule.replace(/-/g, ' ').replace(/^./, (c) => c.toUpperCase());
}
