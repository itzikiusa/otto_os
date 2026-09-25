<script lang="ts">
  import { transcript as transcriptStore } from '../lib/stores/transcript.svelte';
  $effect(() => {
    const onVis = (): void => transcriptStore.setVisible(!document.hidden);
    onVis();
    document.addEventListener('visibilitychange', onVis);
    return () => {
      document.removeEventListener('visibilitychange', onVis);
      transcriptStore.reset();
    };
  });
  import { untrack } from 'svelte';
  // Main shell (post-auth): rail/navigator + tab bar + module router +
  // right panel + status bar + palette + global keys.
  //
  // Special case: `router.module === 's'` is the guest share view — it renders
  // SharePage full-screen and skips all the usual shell chrome entirely.
  import SharePage from '../modules/share/SharePage.svelte';
  import Rail from './Rail.svelte';
  import Navigator from './Navigator.svelte';
  import TabBar from './TabBar.svelte';
  import RightPanel from './RightPanel.svelte';
  import BottomNav from './BottomNav.svelte';
  import Drawer from './Drawer.svelte';
  import NavButtons from './NavButtons.svelte';
  import MobileActionBar from './MobileActionBar.svelte';
  import Icon from '../lib/components/Icon.svelte';
  import StatusBar from './StatusBar.svelte';
  import Palette from './Palette.svelte';
  import FloatingBar from '../lib/components/FloatingBar.svelte';
  import { barStore } from '../lib/stores/bar.svelte';
  import ShortcutsOverlay from './ShortcutsOverlay.svelte';
  import Handover from '../modules/agents/Handover.svelte';
  import AttachIssue from '../modules/agents/AttachIssue.svelte';
  import AttachProductStory from '../modules/agents/AttachProductStory.svelte';
  import { confirmer } from '../lib/confirm.svelte';
  import BroadcastModal from '../lib/components/BroadcastModal.svelte';
  import NotificationBell from './NotificationBell.svelte';
  import { serviceHealth } from '../lib/stores/serviceHealth.svelte';
  import AgentsPage from '../modules/agents/AgentsPage.svelte';
  import HomePage from '../modules/home/HomePage.svelte';
  import { HistoryPage } from '../modules/agents/history';
  import NewSession from '../modules/agents/NewSession.svelte';
  import NewWorkspace from '../modules/settings/NewWorkspace.svelte';
  import ConfirmDialog from '../lib/components/ConfirmDialog.svelte';
  import ContextMenu from '../lib/components/ContextMenu.svelte';
  import FindInPage from '../lib/components/FindInPage.svelte';
  import { findInPage } from '../lib/findinpage.svelte';
  import GitPage from '../modules/git/GitPage.svelte';
  import ApiPage from '../modules/api/ApiPage.svelte';
  import DatabasePage from '../modules/database/DatabasePage.svelte';
  import BrokersPage from '../modules/brokers/BrokersPage.svelte';
  import McpPage from '../modules/mcp/McpPage.svelte';
  import WorkflowsPage from '../modules/workflows/WorkflowsPage.svelte';
  import SkillsLabPage from '../modules/skills-lab/SkillsLabPage.svelte';
  import UsagePage from '../modules/usage/UsagePage.svelte';
  import Settings from '../modules/settings/Settings.svelte';
  import Walkthroughs from '../modules/help/Walkthroughs.svelte';
  import { GUIDES } from '../modules/help/sections';
  import { availableSections, groupLabel as settingsGroupLabel } from '../modules/settings/sections';
  import ProductPage from '../modules/product/ProductPage.svelte';
  import CanvasPage from '../modules/canvas/CanvasPage.svelte';
  import DesignHallPage from '../modules/design-hall/DesignHallPage.svelte';
  import InsightsPage from '../modules/insights/InsightsPage.svelte';
  import MissionControlPage from '../modules/mission-control/MissionControlPage.svelte';
  import SwarmPage from '../modules/swarm/SwarmPage.svelte';
  import LoopsPage from '../modules/loops/LoopsPage.svelte';
  import ProofPage from '../modules/proof/ProofPage.svelte';
  import ScheduledTasksPage from '../modules/scheduled-tasks/ScheduledTasksPage.svelte';
  import PersonalAgentsPage from '../modules/personal-agents/PersonalAgentsPage.svelte';
  import AssistantPage from '../modules/assistant/AssistantPage.svelte';
  import AwsPage from '../modules/aws/AwsPage.svelte';
  import KubernetesPage from '../modules/kubernetes/KubernetesPage.svelte';
  import RunWithOttoPage from '../modules/run-with-otto/RunWithOttoPage.svelte';
  import VaultPage from '../modules/vault/VaultPage.svelte';
  import { BrowserView } from '../modules/browser';
  import SnipEditor from '../modules/snip/SnipEditor.svelte';
  import PluginFrame from '../modules/plugins/PluginFrame.svelte';
  import { plugins } from '../lib/stores/plugins.svelte';
  import { router } from '../lib/router.svelte';
  import { startSnip } from '../lib/snip';
  import { ui, isTauri } from '../lib/stores/ui.svelte';
  import { startWindowDrag } from '../lib/windowDrag';
  import { isPopout, isEmbedded, popoutTitle, openPopout, currentRoute } from '../lib/desktop';
  import SidePane from './SidePane.svelte';
  import AgentDrivingBar from '../lib/components/AgentDrivingBar.svelte';
  import { uiControl } from '../lib/stores/uiControl.svelte';
  // Agent UI control: every module's `ui_*` handlers, registered before the
  // events socket introduces this document (`hello.capabilities`).
  import '../lib/uiCommands/index';
  import SplitDivider from './SplitDivider.svelte';
  import { sidePane } from '../lib/stores/sidePane.svelte';
  import { startGuest, postToHost } from '../lib/embedGuest';
  import { embeddedKeyTarget, paneKey, routeOf, clampShare } from '../lib/sidePane';
  import { ctxMenu } from '../lib/contextmenu.svelte';
  import { viewport } from '../lib/stores/viewport.svelte';
  import { ws } from '../lib/stores/workspace.svelte';
  // Old bookmarks lead to the workspace's existing context editor.
  $effect(() => { if (router.module === 'projects') router.go('settings/context-soul'); });
  import { git } from '../lib/stores/git.svelte';
  import { auth } from '../lib/stores/auth.svelte';
  import { events } from '../lib/events.svelte';
  import { installKeyMap, keyContext, type KeyAction } from '../lib/keys';
  import { attachMenuBridge, attachCloseHandler, handleMenu } from '../lib/menu';
  import { gcWindowKeys } from '../lib/win';
  import { openExternal, isExternalUrl } from '../lib/external';
  import { registry, type Command } from '../lib/commands.svelte';
  import { activeNavId, availableModules, groupLabel, moduleLabel } from '../lib/sidebar';
  import { api, baseUrl } from '../lib/api/client';
  $effect(() => {transcriptStore.setIdentity(JSON.stringify([baseUrl(),auth.me?.id ?? '']));});
  import type { Connection, Session } from '../lib/api/types';
  import { toasts } from '../lib/toast.svelte';
  import { now } from '../lib/stores/now.svelte';

  const moduleName = $derived(router.module === '' ? 'agents' : router.module);

  // Native sidebar vibrancy (desktop shell): the window has an NSVisualEffect
  // view behind the page, so the document goes transparent and ONLY chrome
  // lets it through — the sidebar's `.sidebar-material` 78% tint and the
  // titlebar strips — while `.center` / right panel / status bar stay opaque.
  // Off for the full-screen routes (snip editor, share view) and under
  // reduced transparency (CSS below).
  const vibrant = $derived(
    isTauri &&
      !isEmbedded &&
      (viewport.isDesktop || isPopout) &&
      router.module !== 'snip' &&
      router.module !== 's',
  );
  $effect(() => {
    document.documentElement.classList.toggle('otto-vibrant', vibrant);
    return () => document.documentElement.classList.remove('otto-vibrant');
  });
  // Phone top-bar title: the registry label ("Mission Control", "Skills Lab"),
  // a plugin's own name, never the raw route id.
  const moduleTitle = $derived(
    moduleName === 'plugin'
      ? (plugins.list.find((p) => p.slug === router.parts[1])?.name ?? 'Plugin')
      : moduleLabel(moduleName),
  );
  // The phone Navigator drawer closes on every navigation (tapping a module or
  // a session row in it should land on that page, not leave it covered).
  $effect(() => {
    void router.parts.join('/');
    ui.navDrawerOpen = false;
  });

  // The right activity panel (Git/Files/Notes/Activity/Info/Browser/API) is only
  // meaningful for coding-agent sessions. Connection terminals (SSH / DB / custom,
  // kind === 'connection') are opened from the Connections page but run in the
  // Agents view; they don't need the panel — so gate it on the focused session
  // actually being an agent session.
  const showRightPanel = $derived(moduleName === 'agents' && ws.activeSession?.kind === 'agent');

  // Load the runtime plugin list once authenticated (drives the sidebar). Reads
  // auth.phase only; the write to plugins.list isn't read here, so no loop.
  $effect(() => {
    if (auth.phase === 'ready') {
      plugins.load();
    }
  });

  // ---- impersonation countdown ---
  // The admin token is saved in localStorage with a timestamp so we can show a
  // 30-minute countdown from when the impersonation started. If no timestamp is
  // stored we treat the start as "now" (conservative: 30 min from this load).
  const IMP_START_KEY = 'otto_imp_start_ms';
  const IMP_DURATION_MS = 30 * 60 * 1000;

  $effect(() => {
    if (auth.isImpersonating && !localStorage.getItem(IMP_START_KEY)) {
      localStorage.setItem(IMP_START_KEY, String(Date.now()));
    }
    if (!auth.isImpersonating) {
      localStorage.removeItem(IMP_START_KEY);
    }
  });

  const impSecsLeft = $derived.by(() => {
    if (!auth.isImpersonating) return 0;
    void now(); // reactive tick
    const startMs = parseInt(localStorage.getItem(IMP_START_KEY) ?? '0', 10) || Date.now();
    const elapsed = Date.now() - startMs;
    return Math.max(0, Math.ceil((IMP_DURATION_MS - elapsed) / 1000));
  });

  function fmtImpCountdown(s: number): string {
    if (s <= 0) return '0:00';
    const m = Math.floor(s / 60);
    const sec = s % 60;
    return `${m}:${sec.toString().padStart(2, '0')}`;
  }

  // ---- route → store sync (one-way; avoids route↔store loop) ----
  // When the URL is `#/agents/<sessionId>` (set by navigateToSession or Back/Forward),
  // apply the session id into the workspace store's pane/tab state. This is
  // intentionally ONE-WAY: the store mutation (openSession) does NOT re-navigate,
  // so there is no loop. The router's own `navigating` flag guards against
  // recursive hash changes during back/forward.
  $effect(() => {
    const sessionId = router.parts[1];
    const module = router.module;
    if (module === 'agents' && sessionId) {
      // React ONLY to route changes. `activeSessionId` is read untracked: it
      // changes whenever you focus a different pane (focusedPane moves), and if
      // this effect tracked it, focusing pane B would re-fire here with the URL
      // still on pane A and "restore" A into the focused pane — clobbering the
      // click. Reading it untracked lets focusPane move the frame freely; a real
      // route change (tab/navigator/back-forward) still falls through to sync.
      if (untrack(() => ws.activeSessionId) !== sessionId) {
        ws.openSession(sessionId);
      }
    }
  });

  // ---- `?` shortcuts cheat-sheet + focused-session action modals ----
  let shortcutsOpen = $state(false);
  // Session-scoped modals the palette opens against the active session. They
  // mirror the per-pane ⋯ menu but target ws.activeSession globally.
  let sessionAction: { kind: 'handover' | 'attach-issue' | 'attach-product'; sessionId: string } | null =
    $state(null);

  function openSessionAction(kind: 'handover' | 'attach-issue' | 'attach-product'): void {
    const s = ws.activeSession;
    if (!s) return;
    sessionAction = { kind, sessionId: s.id };
  }

  async function renameActiveSession(): Promise<void> {
    const s = ws.activeSession;
    if (!s) return;
    const next = await confirmer.promptText('Rename session', {
      title: 'Rename session',
      confirmLabel: 'Rename',
      initial: s.title,
      placeholder: 'Session name',
    });
    if (!next || next === s.title) return;
    try {
      await ws.renameSession(s.id, next);
    } catch (e) {
      toasts.error('Rename failed', e instanceof Error ? e.message : String(e));
    }
  }

  // ---- mobile action bar helpers ----
  // These are the SAME actions the keyboard map calls (App.svelte:187-270).
  // Exposed as named functions so MobileActionBar can receive them as props
  // without duplicating any logic.
  function mobileOpenPalette(): void {
    if (ui.paletteOpen) ui.paletteOpen = false;
    else ui.openPalette('commands');
  }
  function mobileNewSession(): void {
    ui.newSessionOpen = true;
  }
  function mobileCloseTab(): void {
    ws.closeActiveTab();
  }
  function mobileFind(): void {
    // A focused component (terminal OR the DB query editor) can OWN find via
    // keyContext.openFind — route to its in-component search; else the page-wide
    // find-in-page overlay.
    if (keyContext.openFind) {
      keyContext.openFind();
    } else {
      findInPage.show();
    }
  }
  function mobileBroadcast(): void {
    ui.openBroadcast();
  }

  // `?` (no modifier, not while typing) opens the shortcuts cheat-sheet. The
  // global keymap in lib/keys.ts only handles modifier chords, so `?` lives here.
  $effect(() => {
    function onHelpKey(e: KeyboardEvent): void {
      if (e.key !== '?' || e.metaKey || e.ctrlKey || e.altKey) return;
      // A module that owns `?` (e.g. the Kubernetes workspace) handles it first
      // in the capture phase and marks it consumed.
      if (e.defaultPrevented) return;
      const el = document.activeElement as HTMLElement | null;
      const typing =
        !!el &&
        (el.tagName === 'INPUT' ||
          el.tagName === 'TEXTAREA' ||
          el.isContentEditable ||
          !!el.closest('.cm-editor'));
      if (typing || ui.overlayOpen) return;
      e.preventDefault();
      // In the side-by-side pane the sheet belongs to the window.
      if (isEmbedded) postToHost({ type: 'key', action: 'shortcuts' });
      else shortcutsOpen = true;
    }
    window.addEventListener('keydown', onHelpKey);
    return () => window.removeEventListener('keydown', onHelpKey);
  });

  // ---- boot ----
  $effect(() => {
    void ws.load();
    events.start();
    let unlistenMenu: (() => void) | null = null;
    let unlistenClose: (() => void) | null = null;
    // Once per WINDOW, never in the side-by-side pane (an iframe of this
    // window): window-key GC and the native menu bridge (the host forwards
    // the pane its menu items — a second listener would run each one twice).
    if (!isEmbedded) {
      void gcWindowKeys();
      void attachMenuBridge().then((fn) => (unlistenMenu = fn));
      void attachCloseHandler().then((fn) => (unlistenClose = fn));
    }

    // Suppress the native WKWebView context menu globally, except on editable
    // elements and elements that opt in with .allow-native-menu.
    function suppressNativeMenu(e: MouseEvent): void {
      const target = e.target as Element | null;
      if (!target) return;
      if (
        target.closest('input') ||
        target.closest('textarea') ||
        target.closest('[contenteditable]') ||
        target.closest('.allow-native-menu')
      ) {
        return;
      }
      e.preventDefault();
    }
    window.addEventListener('contextmenu', suppressNativeMenu);

    // External links: a `<a target="_blank">` to an http(s) URL won't reach the
    // system browser inside the Tauri webview, so intercept those clicks and
    // hand them to the shell `open` command. Internal hash routes are untouched.
    function onLinkClick(e: MouseEvent): void {
      if (e.defaultPrevented || e.button !== 0 || e.metaKey || e.ctrlKey) return;
      const a = (e.target as Element | null)?.closest?.('a');
      if (!a) return;
      const href = a.getAttribute('href');
      if (a.target === '_blank' && isExternalUrl(href)) {
        e.preventDefault();
        void openExternal(href);
      }
    }
    window.addEventListener('click', onLinkClick, { capture: true });

    return () => {
      events.stop();
      unlistenMenu?.();
      unlistenClose?.();
      window.removeEventListener('contextmenu', suppressNativeMenu);
      window.removeEventListener('click', onLinkClick, { capture: true });
    };
  });

  // Open Git tabs remain fresh while working in other modules. Only the
  // visible, focused window schedules network work; focus resumes due repos.
  $effect(() => {
    // The side-by-side pane keeps repos fresh only while it shows Git — the
    // main window already polls every open repo tab.
    if (isEmbedded && moduleName !== 'git') return;
    let stopped = false;
    void untrack(() => git.initializeOpenTabs()).then(() => {
      if (!stopped) git.startAutoFetch();
    });
    const wake = (): void => git.requestAutoFetch();
    window.addEventListener('focus', wake);
    document.addEventListener('visibilitychange', wake);
    return () => {
      stopped = true;
      window.removeEventListener('focus', wake);
      document.removeEventListener('visibilitychange', wake);
      git.stopAutoFetch();
    };
  });

  // keep git store in sync with workspace
  $effect(() => {
    if (ws.currentId) void git.loadRepos(ws.currentId);
  });

  // ---- keyboard map ----
  // One dispatcher for the key map, the side-by-side pane (which hands the
  // window its window-level chords) and nothing else.
  function runKeyAction(action: KeyAction | 'shortcuts', index?: number): void {
    switch (action) {
      case 'shortcuts':
        shortcutsOpen = true;
        break;
      case 'toggleSidePane':
        if (sidePane.route !== null && sidePane.showing) sidePane.close();
        else openSidePicker();
        break;
      case 'palette':
        // Desktop: ⌘K focuses the floating bar (the one command surface).
        // Phone/tablet, pop-outs and a hidden bar keep the palette sheet.
        if (ui.paletteOpen) ui.paletteOpen = false;
        else if (barStore.mounted) barStore.requestFocus();
        else ui.openPalette('commands');
        break;
      case 'askOtto':
        ui.openPalette('english');
        break;
      case 'settings':
        router.go('settings/appearance');
        break;
      case 'updateCLIs':
        void updateAllCLIs();
        break;
      case 'snip':
        void startSnip();
        break;
      case 'broadcast':
        ui.openBroadcast();
        break;
      case 'hardReload':
        // Full UI refresh, like a browser hard reload. Sessions, workspaces,
        // and everything else live in the daemon (the active workspace + open
        // panes are restored from localStorage), so nothing is lost — this just
        // re-fetches fresh state and clears any stale in-memory UI.
        window.location.reload();
        break;
      case 'toggleRail':
        ui.toggleRail();
        break;
      case 'toggleRight':
        ui.toggleRight();
        break;
      case 'newSession':
        ui.newSessionOpen = true;
        break;
      case 'closeTab':
        ws.closeActiveTab();
        break;
      case 'reopenTab':
        ws.reopenClosedTab();
        break;
      case 'nextTab':
        ws.cycleTab(1);
        break;
      case 'prevTab':
        ws.cycleTab(-1);
        break;
      case 'nextSession':
        ws.cycleTab(1);
        break;
      case 'prevSession':
        ws.cycleTab(-1);
        break;
      case 'jumpSession':
        if (index) ws.focusSessionByIndex(index);
        break;
      case 'splitVertical':
        ws.split('col');
        break;
      case 'splitHorizontal':
        ws.split('row');
        break;
      case 'find':
        if (keyContext.openFind) {
          keyContext.openFind();
        } else {
          findInPage.show();
        }
        break;
      case 'appZoomIn':
        ui.zoomIn();
        break;
      case 'appZoomOut':
        ui.zoomOut();
        break;
      case 'appZoomReset':
        ui.zoomReset();
        break;
      case 'termZoomIn':
        ui.termZoomIn();
        break;
      case 'termZoomOut':
        ui.termZoomOut();
        break;
      case 'termZoomReset':
        ui.termZoomReset();
        break;
      case 'navBack':
        router.back();
        break;
      case 'navForward':
        router.forward();
        break;
    }
  }
  $effect(() => {
    return installKeyMap((action, _e, index) => {
      if (isEmbedded) {
        // The side pane: pane verbs run here, window verbs in the host.
        const target = embeddedKeyTarget(action, paneKey(router.parts.join('/')));
        if (target === 'host') postToHost({ type: 'key', action, ...(index ? { index } : {}) });
        else if (target === 'close-pane') postToHost({ type: 'close' });
        else runKeyAction(action, index);
        return;
      }
      runKeyAction(action, index);
    });
  });

  // ---- update CLIs helper (shared by palette + any future callers) ----
  async function updateAllCLIs(): Promise<void> {
    const wsId = ws.currentId;
    if (!wsId) { toasts.error('No workspace selected'); return; }
    try {
      const session = await api.post<Session>(`/workspaces/${wsId}/providers/update`, {});
      ws.addSession(session);
      toasts.info('Updating CLIs…', 'Watch the Update CLIs session for progress');
    } catch (e) {
      toasts.error('Update CLIs failed', e instanceof Error ? e.message : String(e));
    }
  }

  // ---- palette commands: core ----
  $effect(() => {
    const unreg = registry.register('core', [
      { id: 'core.new-session', title: 'New session…', group: 'Sessions', shortcut: '⌘T', keywords: 'spawn agent terminal claude codex shell', run: () => (ui.newSessionOpen = true) },
      { id: 'core.new-session-scratch', title: 'New session (no workspace)…', group: 'Sessions', keywords: 'scratch home adhoc workspace-less', run: () => { ui.newSessionScratch = true; ui.newSessionOpen = true; } },
      { id: 'core.ask-otto', title: 'Ask Otto (plain English)', group: 'Sessions', shortcut: '⌘I', keywords: 'orchestrate natural language command free text', run: () => ui.openPalette('english') },
      { id: 'core.broadcast', title: 'Broadcast message to sessions', group: 'Sessions', shortcut: '⌘⇧B', keywords: 'send message every agent tell all selected', run: () => ui.openBroadcast() },
      { id: 'core.close-tab', title: 'Close tab', group: 'Sessions', shortcut: '⌘W', run: () => ws.closeActiveTab() },
      { id: 'core.reopen-tab', title: 'Reopen closed tab', group: 'Sessions', shortcut: '⌘⇧T', keywords: 'restore undo close tab session', run: () => ws.reopenClosedTab() },
      { id: 'core.next-session', title: 'Next session', group: 'Sessions', shortcut: '⌘]', keywords: 'switch tab forward cycle', run: () => ws.cycleTab(1) },
      { id: 'core.prev-session', title: 'Previous session', group: 'Sessions', shortcut: '⌘[', keywords: 'switch tab back cycle', run: () => ws.cycleTab(-1) },
      { id: 'core.split-v', title: 'Split vertically', group: 'Sessions', shortcut: '⌘D', run: () => ws.split('col') },
      { id: 'core.split-h', title: 'Split horizontally', group: 'Sessions', shortcut: '⌘⇧D', run: () => ws.split('row') },
      { id: 'core.new-workspace', title: 'Add workspace…', group: 'Workspaces', keywords: 'create new project folder directory', run: () => (ui.newWorkspaceOpen = true) },
      { id: 'core.update-clis', title: 'Update all CLIs', group: 'Tools', shortcut: '⌘U / ⌘⇧U', keywords: 'upgrade claude codex agy cli version', run: () => void updateAllCLIs() },
      { id: 'core.snip', title: 'Take a screenshot (snip)', group: 'Tools', shortcut: '⌘⇧S', keywords: 'snip screenshot capture screen region annotate clipboard grab shot', run: () => void startSnip() },
      { id: 'core.go-settings', title: 'Open Settings', group: 'Navigate', keywords: 'preferences appearance', run: () => router.go('settings/appearance') },
      { id: 'core.go-walkthroughs', title: 'Open Help', group: 'Navigate', keywords: 'help guide guides readme docs shortcuts keys intro tour film video walkthroughs onboarding', run: () => router.go('walkthroughs') },
      { id: 'core.go-brokers', title: 'Go to Message Brokers', group: 'Navigate', detail: 'Infrastructure', keywords: 'message broker kafka redpanda topic consumer producer partition schema registry avro protobuf', run: () => router.go('brokers') },
      // Canvas lost its sidebar row to Design Hall (it is the Whiteboard studio)
      // but stays a route of its own — keep it one ⌘K away.
      { id: 'core.go-canvas', title: 'Go to Canvas', group: 'Navigate', detail: 'Build · Design Hall whiteboard', keywords: 'canvas whiteboard diagram sketch uml sequence flowchart excalidraw mermaid d2', run: () => router.go('canvas') },
      { id: 'core.toggle-rail', title: 'Toggle sidebar', group: 'View', shortcut: '⌘1', run: () => ui.toggleRail() },
      { id: 'core.toggle-right', title: 'Toggle right panel', group: 'View', shortcut: '⌘J', run: () => ui.toggleRight() },
      ...(isTauri ? [{ id: 'core.open-in-window', title: 'Open in new window', group: 'View', keywords: 'pop out popout detach separate native window', run: () => void openPopout(currentRoute(), moduleLabel(moduleName)).catch((e: unknown) => toasts.error('Could not open window', e instanceof Error ? e.message : String(e))) }] : []),
      { id: 'core.theme-native', title: 'Theme: Native', group: 'Appearance', run: () => ui.setTheme('native') },
      { id: 'core.theme-pro-dark', title: 'Theme: Pro Dark', group: 'Appearance', run: () => ui.setTheme('pro-dark') },
      { id: 'core.theme-warm', title: 'Theme: Warm', group: 'Appearance', run: () => ui.setTheme('warm') },
      { id: 'core.notes', title: 'Open Notes panel', group: 'View', run: () => ui.openRight('notes') },
      { id: 'core.git-panel', title: 'Open Git panel', group: 'View', run: () => ui.openRight('git') },
      { id: 'core.shortcuts', title: 'Keyboard shortcuts', group: 'Help', shortcut: '?', keywords: 'keys cheat sheet bindings hotkeys', run: () => (shortcutsOpen = true) },
      { id: 'core.logout', title: 'Sign out', group: 'Account', run: () => auth.logout() },
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
    return registry.register(
      'nav',
      mods.map((m) => ({
        id: `core.go-${m.id}`,
        title: `Go to ${m.label}`,
        group: 'Navigate',
        detail: groupLabel(m.group),
        keywords: `module ${m.id.replace(/[-/]/g, ' ')} ${groupLabel(m.group)} ${m.keywords ?? ''}`,
        run: () => router.go(m.id),
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
        keywords: 'favorite favourite star pin sidebar current page unfavorite',
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
        title: `Settings: ${s.label}`,
        group: 'Settings',
        detail: settingsGroupLabel(s.group),
        keywords: `settings preferences ${s.id.replace(/-/g, ' ')} ${s.keywords ?? ''}`,
        run: () => router.go(`settings/${s.id}`),
      })),
    );
  });

  // ---- palette commands: Help guides ----
  // One "Guide: <title>" per README in modules/help/sections (static, bundled
  // at build time), so any guide is ⌘K away from anywhere in the app.
  $effect(() =>
    registry.register(
      'guides',
      GUIDES.map((g) => ({
        id: `help.guide.${g.id}`,
        title: `Guide: ${g.title}`,
        group: 'Help',
        detail: g.group,
        keywords: `help guide readme docs ${g.id.replace(/-/g, ' ')} ${g.summary} ${g.shortcuts.join(' ')}`,
        run: () => router.go(`walkthroughs/${g.id}`),
      })),
    ),
  );

  // ---- palette commands: focused session ----
  // Lifecycle verbs for the currently-active session (mirrors the per-pane ⋯
  // menu, which is otherwise undiscoverable). Registered only when a session is
  // focused; the closures act on ws.activeSession at run time. Hand over is
  // agent-only, matching the ⋯ menu.
  $effect(() => {
    const active = ws.activeSession;
    if (!active) return registry.register('focused-session', []);
    const isAgent = active.kind === 'agent';
    const cmds = [
      { id: 'focus.restart', title: 'Restart focused session', group: 'Session', keywords: 'reload reboot relaunch active current', run: () => void ws.requestRestart(active.id) },
      { id: 'focus.archive', title: 'Archive focused session', group: 'Session', keywords: 'close hide stash active current', run: () => void ws.archiveSession(active.id) },
      { id: 'focus.rename', title: 'Rename focused session…', group: 'Session', keywords: 'title name active current', run: () => void renameActiveSession() },
      ...(isAgent
        ? [{ id: 'focus.handover', title: 'Hand over focused session…', group: 'Session', keywords: 'handoff transfer pass context active current', run: () => openSessionAction('handover') }]
        : []),
      { id: 'focus.attach-issue', title: 'Attach Jira issue to focused session…', group: 'Session', keywords: 'jira ticket link story active current', run: () => openSessionAction('attach-issue') },
      { id: 'focus.attach-product', title: 'Attach product story to focused session…', group: 'Session', keywords: 'product story link context active current', run: () => openSessionAction('attach-product') },
    ];
    return registry.register('focused-session', cmds);
  });

  // ---- palette commands: workspaces ----
  $effect(() => {
    const unreg = registry.register(
      'workspaces',
      ws.workspaces.map((w) => ({
        id: `ws.${w.id}`,
        title: `Switch workspace: ${w.name}`,
        group: 'Workspaces',
        keywords: w.root_path,
        run: () => void ws.select(w.id),
      })),
    );
    return unreg;
  });

  // ---- palette commands: sessions ----
  // Apply the persisted zoom via native WKWebView page-zoom in Tauri (crisp;
  // CSS `zoom` would blur the terminal). Re-runs whenever the zoom changes.
  $effect(() => {
    void ui.zoom; // track
    void ui.applyNativeZoom();
  });

  $effect(() => {
    // Archived sessions are parked (restore them from the sidebar's Archived
    // list); "Focus" on one opened a dead tab. Same rule as the ⌥Space bar.
    const unreg = registry.register(
      'sessions',
      ws.sessions.filter((s) => !s.archived).map((s) => ({
        id: `session.${s.id}`,
        title: `Focus session: ${s.title}`,
        group: 'Sessions',
        keywords: s.provider,
        run: () => {
          ws.navigateToSession(s.id);
        },
      })),
    );
    return unreg;
  });

  // ---- palette commands: repos ("open repo <name>") ----
  $effect(() => {
    return registry.register(
      'repos',
      git.repos.map((r) => ({
        id: `repo.${r.id}`,
        title: `Open repo: ${r.name}`,
        group: 'Git',
        keywords: `repository ${r.path}`,
        run: () => router.go(`git/${r.id}`),
      })),
    );
  });

  // ---- palette commands: connections ("connect <name>") ----
  $effect(() => {
    const wsId = ws.currentId;
    // The side pane's palette is the window's (its commands are mirrored).
    if (!wsId || isEmbedded) return;
    let cancelled = false;
    let unreg: (() => void) | null = null;
    void api
      .get<Connection[]>(`/workspaces/${wsId}/connections`)
      .then((conns) => {
        if (cancelled) return;
        unreg = registry.register(
          'connections',
          conns.map((c) => ({
            id: `connect.${c.id}`,
            title: `Connect: ${c.name}`,
            group: 'Connections',
            keywords: `${c.kind} open`,
            run: async () => {
              const session = await api.post<Session>(`/connections/${c.id}/open`, {});
              ws.addSession(session);
              toasts.success('Connection opened', c.name);
            },
          })),
        );
      })
      .catch(() => {});
    return () => {
      cancelled = true;
      unreg?.();
    };
  });

  // ---- side by side ----------------------------------------------------
  // The content column splits into the main pane and a side pane (another
  // document: lib/sidePane.ts, stores/sidePane.svelte.ts). Desktop main
  // window only; the entry points are the sidebar rows' context menu and
  // ⌥-click, ⌘\ and the ⌘K commands below.
  const splitModules = $derived.by(() => {
    const pluginEntries = plugins.list
      .filter((p) => auth.canPlugin(p.slug, 'view'))
      .map((p) => ({ id: `plugin/${p.slug}`, icon: p.icon, label: p.name }));
    return availableModules((f) => auth.can(f, 'view'), pluginEntries);
  });

  /** A pane's display name for a route: the sidebar label (a plugin's own
   *  name), or the route's name when it has no entry (Database, Settings…). */
  function paneMeta(route: string): { label: string } {
    const key = paneKey(route);
    const first = routeOf(route).split('/')[0] || 'agents';
    const m = splitModules.find((x) => x.id === key);
    return { label: m && (first === key || first === 'plugin') ? m.label : moduleLabel(first) };
  }
  const mainMeta = $derived(paneMeta(router.parts.join('/')));
  const sideMeta = $derived(sidePane.route === null ? null : paneMeta(sidePane.route));

  /** ⌘\ / "Open side pane…": pick a module for the side pane (the main pane's
   *  own module is left out; the pane's current one is checked). */
  function openSidePicker(): void {
    if (!sidePane.supported) return;
    const primary = sidePane.primaryKey;
    const current = sidePane.showing ? sidePane.key : null;
    const items = splitModules
      .filter((m) => m.id !== primary)
      .map((m) => ({
        label: m.label,
        icon: m.icon,
        checked: m.id === current,
        action: () => sidePane.open(m.id, { label: m.label }),
      }));
    ctxMenu.showAt(null, items, { filter: true, filterPlaceholder: 'Open side by side…', maxVisible: 12 });
  }

  /** The pane handed back a route of the main pane's module. A session the
   *  pane just created may not be in this window's list yet — load it first
   *  so the route→store sync can open its tab. */
  async function openInMain(route: string): Promise<void> {
    const m = /^agents\/([^/?]+)/.exec(route);
    if (m && !ws.sessions.some((s) => s.id === m[1])) {
      await ws.refreshSessions().catch(() => {});
    }
    router.go(route);
  }

  // Host: the pane's messages, focus tracking and the router delegate.
  $effect(() =>
    sidePane.listen({
      runKey: (action, index) => runKeyAction(action as KeyAction | 'shortcuts', index),
      openInMain: (route) => void openInMain(route),
      selectWorkspace: (id) => {
        if (ws.currentId !== id && ws.workspaces.some((w) => w.id === id)) void ws.select(id);
      },
    }),
  );

  // Host → pane: which module the main pane shows, and its workspace (the
  // pane follows a workspace switch).
  $effect(() => {
    void sidePane.primaryKey;
    void sidePane.placement;
    void ui.railExpanded;
    const wsId = ws.currentId;
    if (!sidePane.showing || sidePane.status !== 'ready') return;
    sidePane.postHost();
    if (wsId) sidePane.post({ type: 'workspace', id: wsId });
  });

  // The split container's width: the pane only shows while two panes fit.
  let splitWidth = $state(0);
  $effect(() => {
    sidePane.width = splitWidth;
  });
  const sideShare = $derived(clampShare(sidePane.share, splitWidth));

  // ⌘K: the side-by-side verbs.
  $effect(() => {
    if (!sidePane.supported) return registry.register('split', []);
    const cmds: Command[] = [
      { id: 'split.pick', title: 'Open side pane…', group: 'View', shortcut: '⌘\\', keywords: 'split side by side two panes pane module picker compare', run: () => openSidePicker() },
      ...splitModules
        .filter((m) => m.id !== sidePane.primaryKey && m.id !== (sidePane.showing ? sidePane.key : null))
        .map((m) => ({
          id: `split.open-${m.id}`,
          title: `Open ${m.label} side by side`,
          group: 'View',
          detail: groupLabel(m.group),
          keywords: `split side by side pane ${m.id.replace(/[-/]/g, ' ')} ${m.keywords ?? ''}`,
          run: () => sidePane.open(m.id, { label: m.label }),
        })),
    ];
    if (sidePane.showing && sideMeta) {
      const side = sideMeta.label;
      cmds.push(
        { id: 'split.close', title: 'Close side pane', group: 'View', shortcut: '⌘\\', keywords: `split side by side ${side}`, run: () => sidePane.close() },
        { id: 'split.swap', title: 'Swap panes', group: 'View', keywords: `split side by side flip ${side}`, run: () => sidePane.swap() },
        { id: 'split.promote', title: `Open ${side} in main pane`, group: 'View', keywords: 'split side by side promote maximize', run: () => sidePane.promote() },
        { id: 'split.reset', title: 'Reset split to 50/50', group: 'View', keywords: 'split side by side divider equal half', run: () => sidePane.resetSplit() },
        sidePane.focused
          ? { id: 'split.focus-main', title: `Focus ${mainMeta.label} (main pane)`, group: 'View', keywords: 'split pane switch', run: () => sidePane.focusMain() }
          : { id: 'split.focus-side', title: `Focus ${side} (side pane)`, group: 'View', keywords: 'split pane switch', run: () => sidePane.focusPane() },
      );
    }
    return registry.register('split', cmds);
  });

  // ⌘K: the side pane's own commands, mirrored (they run in the pane). Ids the
  // window already has — the shell registers the same set in both documents —
  // stay the window's.
  $effect(() => {
    const remote = sidePane.showing ? sidePane.commands : [];
    const label = sideMeta?.label ?? 'Side pane';
    const own = new Set(untrack(() => registry.all).filter((c) => !c.id.startsWith('side:')).map((c) => c.id));
    return registry.register(
      'side-pane-commands',
      remote
        .filter((c) => !own.has(c.id))
        .map((c) => ({
          ...c,
          id: `side:${c.id}`,
          detail: c.detail ? `${c.detail} · ${label}` : label,
          run: () => sidePane.post({ type: 'run-command', id: c.id }),
        })),
    );
  });

  // Both documents: an appearance change made in one pane (a Settings pane)
  // reaches the other; the pane NOT showing Agents adopts the tabs the other
  // one changed (they share this window's keys).
  $effect(() => {
    if (!isEmbedded && sidePane.route === null) return;
    const onStorage = (e: StorageEvent): void => {
      if (e.storageArea !== localStorage) return;
      if (ui.reloadAppearance(e.key)) return;
      if (ws.ownsLayoutKey(e.key) && paneKey(router.parts.join('/')) !== 'agents') ws.adoptPersistedLayout();
    };
    window.addEventListener('storage', onStorage);
    return () => window.removeEventListener('storage', onStorage);
  });

  // ---- the side pane's own half (embedded document only) ----
  $effect(() => {
    if (!isEmbedded) return;
    // A fresh pane never opens on a leftover right panel; ⌘J still toggles it.
    ui.rightOpen = false;
    const stop = startGuest({
      runMenu: (id) => handleMenu(id),
      selectWorkspace: (id) => {
        if (ws.currentId !== id) void ws.select(id);
      },
      runCommand: (id) => void registry.all.find((c) => c.id === id)?.run(),
    });
    postToHost({ type: 'ready', route: currentRoute() });
    return stop;
  });
  $effect(() => {
    if (!isEmbedded) return;
    void router.parts.join('/');
    postToHost({ type: 'route', route: currentRoute() });
  });
  $effect(() => {
    const id = ws.currentId;
    if (isEmbedded && id) postToHost({ type: 'workspace', id });
  });
  $effect(() => {
    if (!isEmbedded) return;
    const list = registry.all.map(({ id, title, group, detail, keywords, shortcut }) => ({ id, title, group, detail, keywords, shortcut }));
    const t = setTimeout(() => postToHost({ type: 'commands', list }), 150);
    return () => clearTimeout(t);
  });
</script>

{#if router.module === 's'}
  <!-- Guest share view: full-screen terminal, no shell chrome. -->
  <SharePage sessionId={router.parts[1] ?? ''} />
{:else if router.module === 'snip'}
  <!-- Snip annotation editor: full-screen canvas, no shell chrome (it gets its
       own Tauri window; in a browser it takes over the current one). Stays
       behind the login gate — the image fetch needs the bearer token. Keyed by
       id: the image loads on mount, so an unkeyed editor surviving a
       snip→snip route change would keep drawing (and auto-copying!) the OLD
       image onto the new snip. -->
  {#key router.parts[1]}
    <SnipEditor />
  {/key}
  <!-- Its "Delete…" asks first — the shell's own dialog host isn't mounted here. -->
  <ConfirmDialog />
{:else}
<!-- Center column: banners + (agents) TabBar + the module
     router. Extracted to a snippet so the desktop 3-pane and the mobile
     single-pane shells render byte-for-byte identical content — only the
     surrounding chrome differs by viewport. -->
{#snippet centerContent()}
  {#if auth.isImpersonating}
    <div class="provider-banner impersonation-banner" role="alert">
      <span>
        Acting as <strong>{auth.me?.username ?? '…'}</strong>
        <span class="imp-real"> (you are <strong>{auth.realUser?.username ?? '…'}</strong>)</span>
      </span>
      <span class="grow"></span>
      {#if impSecsLeft > 0}
        <span class="imp-countdown" class:imp-urgent={impSecsLeft < 120}>
          {fmtImpCountdown(impSecsLeft)} remaining
        </span>
      {:else}
        <span class="imp-countdown imp-urgent">Session may have expired</span>
      {/if}
      <button
        class="pb-dismiss"
        onclick={() => {
          void auth.stopImpersonating().catch((e: unknown) => {
            toasts.error('Could not exit impersonation', e instanceof Error ? e.message : String(e));
          });
        }}
      >Stop impersonating</button>
    </div>
  {/if}
  {#if serviceHealth.visible}
    <div class="provider-banner" role="alert">
      <Icon name="warning" size={14} />
      <span>
        A remote git provider (GitHub / Bitbucket / GitLab) is failing with a
        <strong>gateway error</strong> — it may be down or under maintenance. Your local work is
        unaffected; retries will resume automatically.
      </span>
      <span class="grow"></span>
      <button class="pb-dismiss" onclick={() => serviceHealth.dismiss()} aria-label="Dismiss notice" title="Dismiss">
        <Icon name="x" size={12} />
      </button>
    </div>
  {/if}
  {#if moduleName === 'agents'}
    <TabBar />
  {/if}
  {#if !uiControl.barOwner}
    <!-- Pages without a PageHeader (Agents, Browser…): the agent-driving
         strip sits above the page instead of under its toolbar. -->
    <AgentDrivingBar />
  {/if}
  <div class="content">
    {#if moduleName === 'agents'}
      <AgentsPage />
    {:else if moduleName === 'home'}
      <HomePage />
    {:else if moduleName === 'assistant'}
      <!-- Otto Assistant: threads (Spaces 01–04 + Recent) and a chat rendered
           from the CLI transcript, with Tasks · Memory · Permissions tabs. -->
      <AssistantPage />
    {:else if moduleName === 'history'}
      <!-- Past agent sessions (Otto rows + transcripts found on disk) with a
           read-only conversation view. `#/history`, not `#/agents/…`, whose
           second segment is a session id. -->
      <HistoryPage />
    {:else if moduleName === 'mission-control'}
      <MissionControlPage />
    {:else if moduleName === 'connections' || moduleName === 'database'}
      <!-- The unified Connections hub IS the DB workbench page: its sidebar tree
           holds every profile kind + Kafka clusters; `#/database` stays as an
           alias so existing links/opens keep working. -->
      <DatabasePage />
    {:else if moduleName === 'git'}
      <GitPage />
    {:else if moduleName === 'api'}
      <ApiPage />
    {:else if moduleName === 'brokers'}
      <BrokersPage />
    {:else if moduleName === 'mcp'}
      <McpPage />
    {:else if moduleName === 'workflows'}
      <WorkflowsPage />
    {:else if moduleName === 'scheduled-tasks'}
      <ScheduledTasksPage />
    {:else if moduleName === 'personal-agents'}
      <PersonalAgentsPage />
    {:else if moduleName === 'aws'}
      <AwsPage />
    {:else if moduleName === 'kubernetes'}
      <KubernetesPage />
    {:else if moduleName === 'run-with-otto'}
      <RunWithOttoPage />
    {:else if moduleName === 'skills-eval'}
      <SkillsLabPage />
    {:else if moduleName === 'usage'}
      <UsagePage />
    {:else if moduleName === 'settings'}
      <Settings />
    {:else if moduleName === 'walkthroughs'}
      <Walkthroughs />
    {:else if moduleName === 'product'}
      <ProductPage />
    {:else if moduleName === 'design'}
      <!-- Design Hall: one library for every studio (#/design…). Canvas below
           stays routable as its Whiteboard studio. -->
      <DesignHallPage />
    {:else if moduleName === 'canvas'}
      <CanvasPage />
    {:else if moduleName === 'insights'}
      <InsightsPage />
    {:else if moduleName === 'swarm'}
      <SwarmPage />
    {:else if moduleName === 'loops'}
      <LoopsPage />
    {:else if moduleName === 'proof'}
      <ProofPage />
    {:else if moduleName === 'vault'}
      <VaultPage />
    {:else if moduleName === 'browser'}
      <BrowserView />
    {:else if moduleName === 'plugin'}
      {#if router.parts[1]}
        {#key router.parts[1]}
          <PluginFrame slug={router.parts[1]} />
        {/key}
      {:else}
        <AgentsPage />
      {/if}
    {:else}
      <AgentsPage />
    {/if}
  </div>
{/snippet}

{#if viewport.isDesktop || isPopout || isEmbedded}
<!-- DESKTOP (≥1025px): the original, unchanged 3-pane shell. A pop-out window
     (`?popout=1`, desktop shell `open_popout`) renders the same shell at any
     width minus the sidebar + status bar, under a slim unified title strip. -->
<!-- App zoom: Tauri uses the native WKWebView page-zoom (applyNativeZoom). In a
     BROWSER we used to apply CSS `zoom:${ui.zoom}` here, but CSS zoom (a) stretches
     the WebGL terminal canvas (oversized + clipped fit) and (b) breaks click
     hit-testing + absolutely-positioned popover/dropdown coordinates. So in the
     browser we DON'T CSS-zoom — users scale crisply with the browser's own zoom
     (⌘+/−), which re-rasterizes everything (terminal included) and keeps
     coordinates correct. ui.zoom still drives native zoom inside Tauri. -->
<div class="shell" class:vibrant class:embedded={isEmbedded}>
  {#if isPopout && isTauri && !isEmbedded}
    <!-- Pop-out title strip: the overlaid traffic lights sit in it and it
         drags the window (double-click zooms), like a unified title bar. -->
    <!-- svelte-ignore a11y_no_static_element_interactions -->
    <div class="popout-titlebar sidebar-material" data-tauri-drag-region onmousedown={startWindowDrag}>
      <span class="popout-title">{popoutTitle()}</span>
    </div>
  {/if}
  <div class="shell-main">
    {#if !isPopout && !isEmbedded}
    <div class="sidebar" class:tauri-top={isTauri}>
      <!-- Draggable titlebar strip over the overlaid traffic-lights inset, so the
           window can be moved by dragging the top-left (the native title bar is
           hidden by `titleBarStyle: Overlay`). The empty 26px inset has no
           interactive content, so this never steals clicks. -->
      {#if isTauri}
        <!-- svelte-ignore a11y_no_static_element_interactions -->
        <div class="titlebar-drag" data-tauri-drag-region onmousedown={startWindowDrag}></div>
      {/if}
      {#if ui.railExpanded}
        <Navigator />
      {:else}
        <Rail />
      {/if}
    </div>
    {/if}

    <!-- Side by side: the content column holds the main pane and, when one
         is open, a divider + the side pane (another document, SidePane.svelte).
         Swap flips their places with CSS order only, so neither reloads. -->
    <div
      class="split"
      class:swapped={sidePane.showing && sidePane.placement === 'leading'}
      bind:clientWidth={splitWidth}
      style:--side-share={sideShare}
    >
      <div class="primary-pane">
        <div class="primary-row">
          <div class="center">
            {@render centerContent()}
            {#if !isPopout && !isEmbedded && viewport.isDesktop}
              <!-- The floating "Type or speak… ⌘K" bar (layout.md §7): bottom-
                   centre over the content column, docking into the status bar
                   while you scroll or type elsewhere. -->
              <FloatingBar host="app" />
            {/if}
          </div>

          <!-- Right panel (Activity/Git/Files/…) for the focused session. Shown in
               every Agents layout — tabbed, split, AND tiled — so per-session activity
               stays visible in multi-session views (it tracks `ws.activeSession`, the
               focused pane/tile), not just when a single session is on screen. It
               belongs to the Agents pane, so a side-by-side split keeps it there. -->
          {#if showRightPanel}
            <RightPanel />
          {/if}
        </div>
      </div>
      {#if sidePane.showing && sideMeta}
        <SplitDivider label={`${mainMeta.label} and ${sideMeta.label}`} />
        <SidePane label={sideMeta.label} />
      {/if}
    </div>
  </div>

  {#if !isPopout && !isEmbedded}
    <StatusBar />
  {/if}
</div>
{:else}
<!-- MOBILE (phone ≤640px / tablet 641–1024px): single-pane content with the
     Navigator and RightPanel moved off-canvas into drawers, a compact top bar,
     and (phone only) a bottom nav. Tablet keeps a persistent narrow Navigator
     column instead of the left drawer. -->
<div class="shell mobile" class:tablet={viewport.isTablet}>
  <!-- svelte-ignore a11y_no_static_element_interactions -->
  <header
    class="mtopbar"
    class:tauri-top={isTauri}
    data-tauri-drag-region
    onmousedown={isTauri ? startWindowDrag : undefined}
  >
    {#if viewport.isPhone}
      <button
        class="mtop-btn"
        onclick={() => (ui.navDrawerOpen = !ui.navDrawerOpen)}
        title="Menu"
        aria-label="Open navigator"
      >
        <Icon name="sidebar" size={18} />
      </button>
    {/if}
    <!-- Back/Forward: visible whenever there is history to walk. Placed left of
         the title so the thumb can reach them comfortably on phone + tablet. -->
    <NavButtons />
    <span class="mtop-title">{moduleName === 'agents' ? (ws.activeSession?.title ?? 'Agents') : moduleTitle}</span>
    <span class="grow"></span>
    <!-- Desktop + tablet reach the bell in the Navigator/Rail; the phone's
         Navigator is a closed drawer, so the top bar carries it instead. -->
    {#if viewport.isPhone}
      <NotificationBell placement="below" />
    {/if}
    {#if showRightPanel}
      <button
        class="mtop-btn"
        class:active={ui.rightOpen}
        onclick={() => ui.toggleRight()}
        title="Activity panel"
        aria-label="Toggle right panel"
      >
        <Icon name="panel" size={18} />
      </button>
    {/if}
  </header>

  <!-- Phone-only quick-action bar: exposes ⌘K/⌘T/⌘W/⌘F/⌘⇧B to touch users
       who can't produce those chords. Wired to the exact same functions the
       keyboard map calls; desktop is completely unaffected. Session verbs, so
       it rides with the Agents page only — elsewhere it was a third chrome row
       (the palette stays one tap away under BottomNav → More). -->
  {#if viewport.isPhone && moduleName === 'agents'}
    <MobileActionBar
      onpalette={mobileOpenPalette}
      onnewSession={mobileNewSession}
      oncloseTab={mobileCloseTab}
      onfind={mobileFind}
      showBroadcast={ws.sessions.length > 0}
      onbroadcast={mobileBroadcast}
    />
  {/if}

  <div class="mbody">
    {#if viewport.isTablet}
      <!-- Persistent narrow Navigator on tablet. -->
      <div class="msidebar" class:tauri-top={isTauri}>
        <Navigator />
      </div>
    {/if}
    <div class="center mcenter">
      {@render centerContent()}
    </div>
  </div>

  {#if viewport.isPhone}
    <BottomNav />
  {/if}

  <!-- The phone already spends a top bar + bottom nav on chrome; the status
       bar only earns its row there when the event stream needs attention. -->
  {#if !viewport.isPhone || events.state !== 'connected'}
    <StatusBar />
  {/if}
</div>

<!-- Phone: Navigator lives in a LEFT drawer with its own open-state
     (ui.navDrawerOpen — not persisted, closed on load, closed on navigation;
     never the desktop sidebar's railExpanded preference). On tablet the
     Navigator is persistent, so no left drawer. -->
{#if viewport.isPhone}
  <Drawer bind:open={ui.navDrawerOpen} side="left" label="Navigator" width="min(86vw, 280px)">
    <Navigator />
  </Drawer>
{/if}

<!-- RightPanel as a RIGHT drawer on phone + tablet (ui.rightOpen). Only
     meaningful in the Agents layout with a focused session. -->
{#if showRightPanel}
  <Drawer bind:open={ui.rightOpen} side="right" label="Activity" width="min(92vw, 360px)">
    <RightPanel forceOpen />
  </Drawer>
{/if}
{/if}

<Palette />

<ShortcutsOverlay open={shortcutsOpen} onclose={() => (shortcutsOpen = false)} />

<!-- Focused-session action modals opened from the palette (mirror SessionView). -->
{#if sessionAction?.kind === 'handover'}
  <Handover sessionId={sessionAction.sessionId} onclose={() => (sessionAction = null)} />
{:else if sessionAction?.kind === 'attach-issue'}
  <AttachIssue sessionId={sessionAction.sessionId} onclose={() => (sessionAction = null)} />
{:else if sessionAction?.kind === 'attach-product'}
  <AttachProductStory sessionId={sessionAction.sessionId} onclose={() => (sessionAction = null)} />
{/if}

{#if ui.broadcastOpen}
  <BroadcastModal />
{/if}

{#if ui.newSessionOpen}
  <NewSession
    initialScratch={ui.newSessionScratch}
    onclose={() => {
      ui.newSessionOpen = false;
      ui.newSessionScratch = false;
    }}
  />
{/if}

{#if ui.newWorkspaceOpen}
  <NewWorkspace onclose={() => (ui.newWorkspaceOpen = false)} />
{/if}

<ConfirmDialog />
<ContextMenu />
<FindInPage />
{/if}

<style>
  .shell {
    /* 100% (of #app), NOT 100vh — in the transparent overlay-titlebar
       WKWebView, 100vh resolves to the full screen height, making the shell
       taller than the window and clipping the bottom row (input, footer,
       status bar) off-screen. */
    height: 100%;
    display: flex;
    flex-direction: column;
    /* The ambient backdrop (tokens.css) lives on the window itself: the
       sidebar's glass blurs it; content columns stay opaque (.center). */
    background-color: var(--bg);
    background-image: var(--ambient-image);
    background-size: cover;
    background-position: center;
  }
  .shell-main {
    flex: 1;
    display: flex;
    min-height: 0;
  }
  .sidebar {
    height: 100%;
    flex-shrink: 0;
    display: flex;
    position: relative;
  }
  .sidebar.tauri-top {
    /* room for overlaid traffic lights in the Tauri window */
    padding-top: 26px;
  }
  /* Window-drag handle filling the empty 26px traffic-lights inset (Tauri only).
     The sidebar's content is padded below it, so this overlays nothing
     interactive — it just lets you move the window from the top-left titlebar. */
  .titlebar-drag {
    position: absolute;
    top: 0;
    inset-inline: 0;
    height: 26px;
    z-index: 5;
  }
  /* Native vibrancy (see `vibrant` in the script): the document is transparent
     so the window's NSVisualEffectView shows through chrome only; content
     columns keep an opaque background. The traffic-lights strip gets the same
     78% tint as `.sidebar-material` so the sidebar reads as one surface. */
  /* The ambient image stays: a Subtle wash is translucent, so the native
     material shows through it; a Wallpaper paints its own opaque base. */
  :global(html.otto-vibrant),
  :global(html.otto-vibrant body),
  .shell.vibrant {
    background-color: transparent;
  }
  .shell.vibrant .titlebar-drag {
    background: var(--glass-tint-native);
  }
  @media (prefers-reduced-transparency: reduce) {
    :global(html.otto-vibrant body),
    .shell.vibrant {
      background-color: var(--bg);
    }
  }
  :global(html.otto-vibrant[data-transparency='reduced'] body),
  :global(html[data-transparency='reduced']) .shell.vibrant {
    background-color: var(--bg);
  }
  /* Pop-out window title strip (unified title bar: traffic lights at the
     start, centred title). */
  .popout-titlebar {
    flex-shrink: 0;
    height: 28px;
    display: flex;
    align-items: center;
    justify-content: center;
    padding-inline: 80px;
    border-block-end: 1px solid var(--separator);
    user-select: none;
  }
  .popout-title {
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    font-size: var(--fs-s);
    font-weight: 600;
    color: var(--text-dim);
  }
  /* ---------- side by side (desktop) ---------- */
  .split {
    flex: 1;
    min-width: 0;
    display: flex;
  }
  .primary-pane {
    flex: 1 1 0;
    min-width: 0;
    display: flex;
    flex-direction: column;
  }
  .primary-row {
    flex: 1;
    min-height: 0;
    display: flex;
  }
  /* A widened right panel never pushes the Agents pane out of its half. */
  .split:has(:global(.side-pane)) .primary-row > :global(.rpanel) {
    max-width: 60%;
  }
  /* Swap: the side pane moves to the leading edge — CSS order only, so
     neither document reloads. */
  .split.swapped > :global(.side-pane) {
    order: -2;
  }
  .split.swapped > :global(.split-divider) {
    order: -1;
  }
  /* The side-by-side pane's own document: opaque page, no ambient art. */
  .shell.embedded {
    background-image: none;
  }
  .center {
    flex: 1;
    min-width: 0;
    display: flex;
    flex-direction: column;
    position: relative;
    /* Content is opaque: the ambient backdrop only ever shows through chrome
       (and on Home, which paints it on purpose — HomePage.svelte). */
    background: var(--bg);
  }

  /* ---------- mobile shell (phone ≤640 / tablet 641–1024) ---------- */
  /* Only mounted when viewport.isDesktop is false, so none of this affects the
     ≥1025px desktop layout above. */
  .mtopbar {
    display: flex;
    align-items: center;
    gap: 6px;
    height: var(--mobile-topbar-h);
    flex-shrink: 0;
    padding: 0 8px;
    border-bottom: 1px solid var(--border);
    background: var(--bg-sidebar);
  }
  .mtopbar.tauri-top {
    height: calc(var(--mobile-topbar-h) + 22px);
    padding-top: 22px;
  }
  .mtop-btn {
    display: grid;
    place-items: center;
    width: 34px;
    height: 34px;
    border: none;
    background: transparent;
    color: var(--text-dim);
    border-radius: var(--radius-s);
    cursor: pointer;
    flex-shrink: 0;
  }
  .mtop-btn:hover {
    background: color-mix(in srgb, var(--text-dim) 14%, transparent);
    color: var(--text);
  }
  .mtop-btn.active {
    color: var(--accent-text);
    background: color-mix(in srgb, var(--accent) 16%, transparent);
  }
  .mtop-title {
    font-size: var(--fs-m);
    font-weight: 600;
    color: var(--text);
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  /* A module page that draws its own PageHeader already titles itself right
     below the bar — don't show the same name twice. */
  :global(.shell.mobile:has(.mcenter [data-testid='page-header'])) .mtop-title {
    display: none;
  }
  .mbody {
    flex: 1;
    display: flex;
    min-height: 0;
  }
  .msidebar {
    height: 100%;
    width: 220px;
    flex-shrink: 0;
    display: flex;
    border-inline-end: 1px solid var(--border);
  }
  .mcenter {
    /* the single content pane fills the remaining width */
    flex: 1;
    min-width: 0;
    /* …and the remaining HEIGHT. Without min-height:0 a flex item refuses to
       shrink below its content's intrinsic height, which breaks the flex height
       chain for nested panes (terminal output, DB results grid, diff viewers),
       collapsing them to ~0 on the narrow mobile shell. Mirrors `.mbody`. */
    min-height: 0;
  }

  /* Upstream/provider outage strip (e.g. Bitbucket 502 / maintenance). */
  .provider-banner {
    display: flex;
    align-items: center;
    gap: 10px;
    padding: 7px 14px;
    font-size: var(--fs-s);
    background: color-mix(in srgb, var(--warning) 18%, var(--surface));
    color: var(--text);
    border-bottom: 1px solid color-mix(in srgb, var(--warning) 45%, transparent);
    z-index: 5;
  }
  .pb-dismiss {
    flex-shrink: 0;
    display: inline-flex;
    align-items: center;
    gap: 4px;
    border: none;
    background: transparent;
    color: var(--text-dim);
    cursor: pointer;
    font-size: var(--fs-s);
    padding: 2px 6px;
    border-radius: 4px;
    line-height: 1;
  }
  .provider-banner > :global(svg) {
    color: var(--warning);
  }
  .pb-dismiss:hover {
    color: var(--text);
    background: color-mix(in srgb, var(--text) 10%, transparent);
  }
  /* Impersonation banner — blue tint to visually differentiate from the
     amber provider-health banner; Stop button is non-destructive styling. */
  .impersonation-banner {
    background: color-mix(in srgb, var(--info) 18%, var(--surface));
    border-bottom-color: color-mix(in srgb, var(--info) 45%, transparent);
  }
  .imp-real {
    font-size: var(--fs-xs);
    color: var(--text-dim);
    margin-inline-start: 4px;
  }
  .imp-countdown {
    font-size: var(--fs-xs);
    color: var(--text-dim);
    font-variant-numeric: tabular-nums;
    flex-shrink: 0;
  }
  .imp-countdown.imp-urgent {
    color: var(--danger);
    font-weight: 600;
  }
  .content {
    flex: 1;
    min-height: 0;
    overflow: hidden;
  }
</style>
