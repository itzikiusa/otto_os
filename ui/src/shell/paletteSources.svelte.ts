// The shell's ⌘K palette sources (architecture-2-05): the core verbs, Go-to
// <module>, the current page's Favorites toggle, Settings sections, Help
// guides, workspaces, sessions, repositories and connections. Moved out of
// App.svelte, which keeps the focused session's verbs (beside the dialogs they
// open), layout and the state it renders. `installShell`
// (./installShell.svelte.ts) installs these once from App's onMount. Each
// source is its own effect, so one source's reads never re-register another's.

import { untrack } from 'svelte';
import { registry } from '../lib/commands.svelte';
import { activeNavId, availableModules, goToEntries, moduleLabel } from '../lib/sidebar';
import { availableSections, groupLabel as settingsGroupLabel } from '../modules/settings/sections';
import { loadConnectionCommands } from '../modules/connections/paletteCommands';
import { repoCommands } from './paletteSources/repos';
import { plugins } from '../lib/stores/plugins.svelte';
import { router } from '../lib/router.svelte';
import { ui, isTauri } from '../lib/stores/ui.svelte';
import { RIGHT_TABS, SIDE_PANEL } from '../lib/rightTabs';
import { rightPanel } from '../lib/stores/rightPanel.svelte';
import { isEmbedded, openPopout, currentRoute } from '../lib/desktop';
import { ws } from '../lib/stores/workspace.svelte';
import { git } from '../lib/stores/git.svelte';
import { auth } from '../lib/stores/auth.svelte';
import { toasts } from '../lib/toast.svelte';
import { toastError } from '../lib/toastError';
import { startSnip } from '../lib/snip';

/** What the sources need from the app root (its local state and helpers). */
export interface PaletteDeps {
  /** The routed module ('agents' for the root route), read at run time. */
  moduleName: () => string;
  openShortcuts: () => void;
  updateAllCLIs: () => Promise<void>;
}

/** Register every shell palette source; returns the teardown (unregisters all). */
export function installPaletteSources(deps: PaletteDeps): () => void {
  return $effect.root(() => {
    // ---- palette commands: core ----
    $effect(() => {
      const unreg = registry.register('core', [
        { id: 'core.new-session', title: 'New session…', group: 'Session', shortcut: '⌘T', keywords: 'spawn agent terminal claude codex shell', run: () => (ui.newSessionOpen = true) },
        { id: 'core.new-session-scratch', title: 'New session (no workspace)…', group: 'Session', keywords: 'scratch home adhoc workspace-less', run: () => { ui.newSessionScratch = true; ui.newSessionOpen = true; } },
        { id: 'core.ask-otto', title: 'Ask Otto (plain English)', group: 'Session', shortcut: '⌘I', keywords: 'orchestrate natural language command free text', run: () => ui.openPalette('english') },
        { id: 'core.broadcast', title: 'Broadcast message to sessions…', group: 'Session', shortcut: '⌘⇧B', keywords: 'send message every agent tell all selected', run: () => ui.openBroadcast() },
        { id: 'core.close-tab', title: 'Close tab', group: 'Session', shortcut: '⌘W', run: () => ws.closeActiveTab() },
        { id: 'core.reopen-tab', title: 'Reopen closed tab', group: 'Session', shortcut: '⌘⇧T', keywords: 'restore undo close tab session', run: () => ws.reopenClosedTab() },
        { id: 'core.next-session', title: 'Next session', group: 'Session', shortcut: '⌘]', keywords: 'switch tab forward cycle', run: () => ws.cycleTab(1) },
        { id: 'core.prev-session', title: 'Previous session', group: 'Session', shortcut: '⌘[', keywords: 'switch tab back cycle', run: () => ws.cycleTab(-1) },
        { id: 'core.split-v', title: 'Split vertically', group: 'Pane', shortcut: '⌘D', run: () => ws.split('col') },
        { id: 'core.split-h', title: 'Split horizontally', group: 'Pane', shortcut: '⌘⇧D', run: () => ws.split('row') },
        { id: 'core.new-workspace', title: 'New workspace…', group: 'Session', keywords: 'create new project folder directory', run: () => (ui.newWorkspaceOpen = true) },
        { id: 'core.update-clis', title: 'Update all CLIs', group: 'Tools', shortcut: '⌘U / ⌘⇧U', keywords: 'upgrade claude codex agy cli version', run: () => void deps.updateAllCLIs() },
        { id: 'core.snip', title: 'Take a screenshot (snip)', group: 'Tools', shortcut: '⌘⇧S', keywords: 'snip screenshot capture screen region annotate clipboard grab shot', run: () => void startSnip() },
        { id: 'core.go-settings', title: 'Go to Settings', group: 'Navigate', shortcut: '⌘,', keywords: 'preferences appearance', run: () => router.go('settings/appearance') },
        { id: 'core.go-back', title: 'Go back', group: 'Navigate', shortcut: '⌘⇧←', keywords: 'previous page history return', run: () => router.back() },
        { id: 'core.go-forward', title: 'Go forward', group: 'Navigate', shortcut: '⌘⇧→', keywords: 'next page history', run: () => router.forward() },
        { id: 'core.toggle-rail', title: 'Toggle sidebar', group: 'View', shortcut: '⌘1', run: () => ui.toggleRail() },
        { id: 'core.toggle-right', title: `Toggle ${SIDE_PANEL}`, group: 'View', shortcut: '⌘J', run: () => ui.toggleRight() },
        ...(isTauri ? [{ id: 'core.open-in-window', title: 'Open in new window', group: 'View', keywords: 'pop out popout detach separate native window', run: () => void openPopout(currentRoute(), moduleLabel(deps.moduleName())).catch((e: unknown) => toastError('Couldn’t open window', e)) }] : []),
        { id: 'core.scheme-auto', title: 'Use automatic color scheme (follow system)', group: 'View', keywords: 'color scheme light dark system automatic mode', run: () => ui.setScheme('auto') },
        { id: 'core.scheme-light', title: 'Use light color scheme', group: 'View', keywords: 'color scheme mode day', run: () => ui.setScheme('light') },
        { id: 'core.scheme-dark', title: 'Use dark color scheme', group: 'View', keywords: 'color scheme mode night', run: () => ui.setScheme('dark') },
        { id: 'core.theme-native', title: 'Use Native theme', group: 'View', run: () => ui.setTheme('native') },
        { id: 'core.theme-pro-dark', title: 'Use Pro Dark theme', group: 'View', run: () => ui.setTheme('pro-dark') },
        { id: 'core.theme-warm', title: 'Use Warm theme', group: 'View', run: () => ui.setTheme('warm') },
        // One "Open <tab> panel" per Side-panel tab (lib/rightTabs.ts). A tab
        // that doesn't exist here (Activity off a session) says so instead.
        ...RIGHT_TABS.map((t) => ({
          id: t.id === 'git' ? 'core.git-panel' : t.id === 'notes' ? 'core.notes' : `core.panel-${t.id}`,
          title: `Open ${t.label} panel`,
          group: 'View',
          detail: SIDE_PANEL,
          keywords: `side session panel right ${t.id}`,
          run: () => {
            if (rightPanel.available(t.id)) ui.openRight(t.id);
            else toasts.info(`${t.label} isn’t available here`, t.sessionOnly ? 'It belongs to an agent session: open one in Agents.' : 'This page already shows it.');
          },
        })),
        { id: 'core.shortcuts', title: 'Show keyboard shortcuts', group: 'Help', shortcut: '?', keywords: 'keys cheat sheet bindings hotkeys', run: deps.openShortcuts },
        { id: 'core.logout', title: 'Sign out', group: 'Tools', keywords: 'log out logout account', run: () => auth.logout() },
      ]);
      return unreg;
    });

    // ---- palette commands: Go to <module> ----
    // Derived from the sidebar registry (RBAC-filtered + permitted plugins), so
    // every module is reachable by ⌘K and the list can never drift from the
    // sidebar. Hidden modules are included on purpose — ⌘K is the way back to
    // one you've hidden. The sidebar section shows as secondary text.
    $effect(() => {
      const pluginEntries = plugins.list
        .filter((p) => auth.canPlugin(p.slug, 'view'))
        .map((p) => ({ id: `plugin/${p.slug}`, icon: p.icon, label: p.name }));
      const mods = availableModules((f) => auth.can(f, 'view'), pluginEntries);
      // Modules + the routes without a sidebar row (Brokers, Canvas, Help) — one
      // derivation shared with the floating bar (lib/sidebar.ts goToEntries).
      return registry.register(
        'nav',
        goToEntries(mods).map((e) => ({
          id: e.id,
          title: e.title,
          group: 'Navigate',
          detail: e.detail,
          keywords: e.keywords,
          run: () => (e.viaModule ? router.openModule(e.route) : router.go(e.route)),
        })),
      );
    });

    // ---- palette commands: Favorites (the current page) ----
    // One toggle for the sidebar entry the current route highlights — "Add Git
    // to Favorites" / "Remove Git from Favorites" — only on a page that has a
    // sidebar entry the user can see (not Settings / Help).
    $effect(() => {
      const id = activeNavId(router.parts);
      const pluginEntries = plugins.list
        .filter((p) => auth.canPlugin(p.slug, 'view'))
        .map((p) => ({ id: `plugin/${p.slug}`, icon: p.icon, label: p.name }));
      const m = availableModules((f) => auth.can(f, 'view'), pluginEntries).find((x) => x.id === id);
      if (!m) return;
      const fav = ui.sidebarFavorites.includes(m.id);
      return registry.register('favorites', [
        {
          id: 'core.toggle-favorite',
          title: fav ? `Remove ${m.label} from Favorites` : `Add ${m.label} to Favorites`,
          group: 'View',
          detail: 'Sidebar',
          keywords: 'favorite favourite star pin sidebar current page unfavorite', // UK search synonym; ui-guards: allow
          run: () => ui.toggleSidebarFavorite(m.id),
        },
      ]);
    });

    // ---- palette commands: Settings sections ----
    // One "Settings: <section>" per section the role can open, generated from
    // the Settings registry (modules/settings/sections.ts) the same way Go-to
    // commands come from the sidebar — a new section is ⌘K-reachable for free.
    $effect(() => {
      const sections = availableSections(auth);
      return registry.register(
        'settings',
        sections.map((s) => ({
          id: `settings.${s.id}`,
          title: `Go to ${s.label} settings`,
          group: 'Settings',
          detail: settingsGroupLabel(s.group),
          keywords: `settings preferences ${s.id.replace(/-/g, ' ')} ${s.keywords ?? ''}`,
          run: () => router.go(`settings/${s.id}`),
        })),
      );
    });

    // ---- palette commands: Help guides ----
    // One "Guide: <title>" per README in modules/help/sections (static, bundled
    // at build time), so any guide is ⌘K away from anywhere in the app. The
    // guides (~360 KB of markdown, parsed at import) are their own chunk,
    // fetched right after the shell mounts rather than in its boot closure.
    $effect(() => {
      let unreg: (() => void) | null = null;
      let stopped = false;
      void import('../modules/help/sections').then(({ GUIDES }) => {
        if (stopped) return;
        unreg = registry.register(
          'guides',
          GUIDES.map((g) => ({
            id: `help.guide.${g.id}`,
            title: `Open the ${g.title} guide`,
            group: 'Help',
            detail: g.group,
            keywords: `help guide readme docs ${g.id.replace(/-/g, ' ')} ${g.summary} ${g.shortcuts.join(' ')}`,
            run: () => router.go(`walkthroughs/${g.id}`),
          })),
        );
      }, () => {});
      return () => {
        stopped = true;
        unreg?.();
      };
    });

    // ---- palette commands: workspaces ----
    $effect(() => {
      const unreg = registry.register(
        'workspaces',
        ws.workspaces.map((w) => ({
          id: `ws.${w.id}`,
          title: `Switch to workspace ${w.name}`,
          group: 'Session',
          keywords: w.root_path,
          run: () => void ws.select(w.id),
        })),
      );
      return unreg;
    });

    // ---- palette commands: sessions ----
    // Only what the commands show: a `session_status` flip (every few seconds
    // per working agent) rewrites ws.sessions but must not re-register the set
    // (and re-rank the ⌘K bar).
    const sessionCmdKey = $derived(
      ws.sessions.map((s) => (s.archived ? '' : `${s.id}\u0001${s.title}\u0001${s.provider}`)).join('\u0002'),
    );
    $effect(() => {
      void sessionCmdKey;
      // Archived sessions are parked (restore them from the sidebar's Archived
      // list); "Focus" on one opened a dead tab. Same rule as the ⌥Space bar.
      const list = untrack(() => ws.sessions);
      const unreg = registry.register(
        'sessions',
        list.filter((s) => !s.archived).map((s) => ({
          id: `session.${s.id}`,
          title: `Focus ${s.title}`,
          detail: 'Session',
          group: 'Session',
          keywords: s.provider,
          run: () => {
            ws.navigateToSession(s.id);
          },
        })),
      );
      return unreg;
    });

    // ---- palette commands: repos ("open repo <name>") ----
    $effect(() => registry.register('repos', repoCommands(git.repos, (route) => router.go(route))));

    // ---- palette commands: connections ("connect <name>") ----
    // Fetched only once ⌘K / the floating bar has been opened in this document
    // (perf F8) — every boot and workspace switch used to pay for the list.
    $effect(() => {
      const wsId = ws.currentId;
      // The side pane's palette is the window's (its commands are mirrored).
      if (!wsId || isEmbedded || !registry.wanted) return;
      let canceled = false;
      let unreg: (() => void) | null = null;
      void loadConnectionCommands(wsId)
        .then((cmds) => {
          if (!canceled) unreg = registry.register('connections', cmds);
        })
        .catch(() => {});
      return () => {
        canceled = true;
        unreg?.();
      };
    });
  });
}
