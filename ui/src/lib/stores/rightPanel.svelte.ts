// The Side panel's context, shared by the panel (shell/RightPanel.svelte), its
// Git / Files tabs and the ⌘K commands:
//   • where it is — inside an agent session (Agents page, agent focused) the
//     Git and Files tabs follow the session's cwd, as they always did; on any
//     other page they work in a workspace folder the user can change (one pick
//     per workspace, persisted);
//   • which tabs exist here — shell tabs minus the session-only ones outside a
//     session (lib/rightTabs.ts), plus the tabs the current page contributes
//     (`register`), so a page with its own details pane (the Vault) folds it
//     into this one panel instead of showing a second, competing right panel.
import type { Component } from 'svelte';
import type { IconName } from '../components/Icon.svelte';
import { contextFolder, parseFolderPicks, prunePicks, shownRightTab, visibleRightTabs, withFolderPick, type PageTabId, type RightTabDef } from '../rightTabs';
import { lsGet, lsSet } from '../storage';
import { router } from '../router.svelte';
import { ui } from './ui.svelte';
import { ws } from './workspace.svelte';

export const LS_PICKS = 'otto_right_folder';

export interface PageTab {
  id: PageTabId;
  label: string;
  icon: IconName;
  // Pages hand in their own panel component (each takes its own props).
  // eslint-disable-next-line @typescript-eslint/no-explicit-any
  component: Component<any>;
  props?: Record<string, unknown>;
  /** The person explicitly showed (true) or hid (false) THIS tab from the
   *  rail or the panel's collapse — the page's own preference to remember. */
  onshow?: (shown: boolean) => void;
}

/** One rail entry: a shell tab or a page tab. */
export interface RailTab {
  id: string;
  label: string;
  icon: IconName;
  page: boolean;
}

class RightPanelStore {
  /** Tabs the mounted page contributes (rendered above the shell tabs). */
  pageTabs: PageTab[] = $state.raw([]);
  /** Folder picked per workspace for the panel outside sessions. */
  picks: Record<string, string> = $state(parseFolderPicks(lsGet(LS_PICKS)));

  /** The Agents page (the bare route is Agents too) with an agent session focused. */
  readonly session = $derived(
    (router.module === '' || router.module === 'agents') && ws.activeSession?.kind === 'agent',
  );
  readonly module = $derived(router.module === '' ? 'agents' : router.module);

  /** Folder the Git and Files tabs use. */
  readonly folder = $derived(
    contextFolder({
      session: this.session,
      sessionCwd: ws.activeSession?.cwd,
      picked: ws.currentId ? this.picks[ws.currentId] : null,
      workspaceRoot: ws.current?.root_path,
    }),
  );
  /** The folder is a pick (not the workspace root): show "Use workspace folder". */
  readonly picked = $derived(!this.session && !!ws.currentId && !!this.picks[ws.currentId]);

  readonly shellTabs: RightTabDef[] = $derived(
    visibleRightTabs({ module: this.module, session: this.session, browserVersion: ui.browserPanelVersion }),
  );
  /** Rail order: the page's own tabs first, then the shell's. */
  readonly tabs: RailTab[] = $derived([
    ...this.pageTabs.map((t) => ({ id: t.id, label: t.label, icon: t.icon, page: true })),
    ...this.shellTabs.map((t) => ({ id: t.id, label: t.label, icon: t.icon, page: false })),
  ]);
  /** What the open panel shows (the chosen tab, or the first one available here). */
  readonly shown = $derived(shownRightTab(ui.rightTabNow, this.tabs.map((t) => t.id)));

  /** Contribute a tab while the page is mounted; returns the unregister. */
  register(tab: PageTab): () => void {
    this.pageTabs = [...this.pageTabs.filter((t) => t.id !== tab.id), tab];
    return () => {
      this.pageTabs = this.pageTabs.filter((t) => t !== tab);
    };
  }

  /** Can `id` be shown here? (⌘K "Open <tab> panel" asks before opening.) */
  available(id: string): boolean {
    return this.tabs.some((t) => t.id === id);
  }

  /** Point the panel at `path` for the current workspace (`null`: its root).
   *  Applied on top of what storage holds now, so another window's pick for
   *  another workspace is never overwritten by this window's stale copy. */
  pickFolder(path: string | null): void {
    const id = ws.currentId;
    if (!id) return;
    this.picks = withFolderPick(parseFolderPicks(lsGet(LS_PICKS)), id, path);
    lsSet(LS_PICKS, JSON.stringify(this.picks));
  }

  /** Another window changed the picks (storage event). */
  reloadPicks(): void {
    this.picks = parseFolderPicks(lsGet(LS_PICKS));
  }

  /** Forget picks of workspaces that no longer exist (`ids`: the live list). */
  prunePicks(ids: readonly string[]): void {
    const stored = parseFolderPicks(lsGet(LS_PICKS));
    const next = prunePicks(stored, ids);
    if (next !== stored) lsSet(LS_PICKS, JSON.stringify(next));
    this.picks = next;
  }
}

export const rightPanel = new RightPanelStore();
