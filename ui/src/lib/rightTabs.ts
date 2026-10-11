// The Side panel's tabs (the right-hand panel, ⌘J) — one list for the panel's
// icon rail (shell/RightPanel.svelte) and the ⌘K "Open <tab> panel" commands
// (shell/App.svelte), so the two never drift. The panel is on every page: in
// an agent session it works on the session's folder, elsewhere on a workspace
// folder the user can change (stores/rightPanel.svelte.ts). Everything here is
// pure (type-only imports) so unit/rightTabs.test.ts runs it under node --test.
import type { IconName } from './components/Icon.svelte';
import type { BrowserPanelVersion, RightTab } from './stores/ui.svelte';

/** The panel's one name: toggle button, drawer, rail and ⌘K all use it. It
 *  was the "Session panel" while it lived only beside agent sessions. */
export const SIDE_PANEL = 'Side panel';

export interface RightTabDef {
  id: RightTab;
  icon: IconName;
  label: string;
  /** Only meaningful beside an agent session (its activity, outputs, canvas
   *  refs, metadata): hidden from the rail everywhere else. */
  sessionOnly?: boolean;
  /** Modules that already ARE this tab: the page and the panel would edit the
   *  same store (API draft, Browser tabs/webviews, the Git page's repo), so
   *  the tab is hidden there instead of showing a second copy. */
  hiddenOn?: readonly string[];
}

export const RIGHT_TABS: readonly RightTabDef[] = [
  { id: 'git', icon: 'branch', label: 'Git', hiddenOn: ['git'] },
  { id: 'files', icon: 'file', label: 'Files' },
  { id: 'notes', icon: 'note', label: 'Notes' },
  { id: 'activity', icon: 'zap', label: 'Activity', sessionOnly: true },
  // Outputs — artifacts the focused agent produced, with sandboxed previews
  // (docs/design/conversation-view.md §5.6).
  { id: 'outputs', icon: 'layers', label: 'Outputs', sessionOnly: true },
  { id: 'canvas', icon: 'shapes', label: 'Canvas', sessionOnly: true },
  { id: 'info', icon: 'info', label: 'Info', sessionOnly: true },
  { id: 'browser', icon: 'globe', label: 'Browser', hiddenOn: ['browser'] },
  { id: 'api', icon: 'send', label: 'API', hiddenOn: ['api'] },
];

/** A page-contributed tab (e.g. the Vault's note details): `page:<name>`. */
export type PageTabId = `page:${string}`;
export const isPageTabId = (id: string): id is PageTabId => id.startsWith('page:');

/** Where the panel is: which module, and whether an agent session is focused. */
export interface RightPanelPlace {
  module: string;
  /** The Agents page with an agent session focused (not a connection terminal). */
  session: boolean;
  browserVersion: BrowserPanelVersion;
}

/** The shell tabs the rail shows here, in rail order. */
export function visibleRightTabs(place: RightPanelPlace): RightTabDef[] {
  return RIGHT_TABS.filter((t) => {
    if (t.sessionOnly && !place.session) return false;
    if (t.hiddenOn?.includes(place.module)) return false;
    // The classic (v1) browser is the per-session panel; outside a session
    // only the Browser module (v2) makes sense.
    if (t.id === 'browser' && place.browserVersion === 'v1' && !place.session) return false;
    return true;
  });
}

/** The tab the open panel shows: the chosen one when it exists here, else the
 *  first one that does. The choice itself is never overwritten, so coming
 *  back to the session (or the Vault) shows Activity (or its details) again. */
export function shownRightTab(active: string, available: readonly string[]): string | null {
  if (available.includes(active)) return active;
  return available[0] ?? null;
}

/** A rail click, VS Code style: another tab opens it; the tab already shown
 *  collapses the panel to the rail. */
export function railClick(open: boolean, shown: string | null, clicked: string): { open: boolean; tab: string } {
  if (open && shown === clicked) return { open: false, tab: clicked };
  return { open: true, tab: clicked };
}

/** Folder the panel's Git and Files tabs work in: the session's cwd inside an
 *  agent session, else the folder picked for this workspace, else its root. */
export function contextFolder(o: {
  session: boolean;
  sessionCwd?: string | null;
  picked?: string | null;
  workspaceRoot?: string | null;
}): string {
  if (o.session) return o.sessionCwd || o.workspaceRoot || '';
  return o.picked || o.workspaceRoot || '';
}

/** Per-workspace folder picks, persisted as `{ [workspaceId]: absPath }`.
 *  Anything malformed is dropped: storage is a convenience, never trusted. */
export function parseFolderPicks(raw: string | null): Record<string, string> {
  try {
    const v: unknown = JSON.parse(raw ?? '{}');
    if (!v || typeof v !== 'object' || Array.isArray(v)) return {};
    const out: Record<string, string> = {};
    for (const [k, p] of Object.entries(v as Record<string, unknown>)) {
      if (typeof p === 'string' && (p.startsWith('/') || p.startsWith('~'))) out[k] = p;
    }
    return out;
  } catch {
    return {};
  }
}

/** Set (`path`) or clear (`null`) one workspace's pick. */
export function withFolderPick(picks: Record<string, string>, wsId: string, path: string | null): Record<string, string> {
  const next = { ...picks };
  if (path) next[wsId] = path;
  else delete next[wsId];
  return next;
}
