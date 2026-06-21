<script lang="ts">
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
  import ShortcutsOverlay from './ShortcutsOverlay.svelte';
  import Handover from '../modules/agents/Handover.svelte';
  import AttachIssue from '../modules/agents/AttachIssue.svelte';
  import AttachProductStory from '../modules/agents/AttachProductStory.svelte';
  import { confirmer } from '../lib/confirm.svelte';
  import BroadcastModal from '../lib/components/BroadcastModal.svelte';
  import NotificationBell from './NotificationBell.svelte';
  import { serviceHealth } from '../lib/stores/serviceHealth.svelte';
  import AgentsPage from '../modules/agents/AgentsPage.svelte';
  import NewSession from '../modules/agents/NewSession.svelte';
  import NewWorkspace from '../modules/settings/NewWorkspace.svelte';
  import ConfirmDialog from '../lib/components/ConfirmDialog.svelte';
  import ContextMenu from '../lib/components/ContextMenu.svelte';
  import FindInPage from '../lib/components/FindInPage.svelte';
  import { findInPage } from '../lib/findinpage.svelte';
  import ConnectionsPage from '../modules/connections/ConnectionsPage.svelte';
  import GitPage from '../modules/git/GitPage.svelte';
  import ApiPage from '../modules/api/ApiPage.svelte';
  import DatabasePage from '../modules/database/DatabasePage.svelte';
  import BrokersPage from '../modules/brokers/BrokersPage.svelte';
  import WorkflowsPage from '../modules/workflows/WorkflowsPage.svelte';
  import SkillsEvalPage from '../modules/skills-eval/SkillsEvalPage.svelte';
  import UsagePage from '../modules/usage/UsagePage.svelte';
  import Settings from '../modules/settings/Settings.svelte';
  import Walkthroughs from '../modules/help/Walkthroughs.svelte';
  import ProductPage from '../modules/product/ProductPage.svelte';
  import InsightsPage from '../modules/insights/InsightsPage.svelte';
  import SwarmPage from '../modules/swarm/SwarmPage.svelte';
  import VaultPage from '../modules/vault/VaultPage.svelte';
  import PluginFrame from '../modules/plugins/PluginFrame.svelte';
  import { plugins } from '../lib/stores/plugins.svelte';
  import { router } from '../lib/router.svelte';
  import { ui, isTauri } from '../lib/stores/ui.svelte';
  import { viewport } from '../lib/stores/viewport.svelte';
  import { ws } from '../lib/stores/workspace.svelte';
  import { git } from '../lib/stores/git.svelte';
  import { auth } from '../lib/stores/auth.svelte';
  import { events } from '../lib/events.svelte';
  import { installKeyMap, keyContext } from '../lib/keys';
  import { attachMenuBridge, attachCloseHandler } from '../lib/menu';
  import { openExternal, isExternalUrl } from '../lib/external';
  import { registry } from '../lib/commands.svelte';
  import { api } from '../lib/api/client';
  import type { Connection, Session } from '../lib/api/types';
  import { toasts } from '../lib/toast.svelte';
  import { now } from '../lib/stores/now.svelte';

  const moduleName = $derived(router.module === '' ? 'agents' : router.module);

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
    if (router.module === 'agents' && sessionId) {
      // Only update if the store doesn't already reflect this session — avoids
      // a spurious pane shuffle when the store and route are already in sync.
      if (ws.activeSessionId !== sessionId) {
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
    if (keyContext.terminalFocused && keyContext.openFind) {
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
      const el = document.activeElement as HTMLElement | null;
      const typing =
        !!el &&
        (el.tagName === 'INPUT' ||
          el.tagName === 'TEXTAREA' ||
          el.isContentEditable ||
          !!el.closest('.cm-editor'));
      if (typing || ui.overlayOpen) return;
      e.preventDefault();
      shortcutsOpen = true;
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
    void attachMenuBridge().then((fn) => (unlistenMenu = fn));
    void attachCloseHandler().then((fn) => (unlistenClose = fn));

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

  // keep git store in sync with workspace
  $effect(() => {
    if (ws.currentId) void git.loadRepos(ws.currentId);
  });

  // ---- keyboard map ----
  $effect(() => {
    return installKeyMap((action, _e, index) => {
      switch (action) {
        case 'palette':
          if (ui.paletteOpen) ui.paletteOpen = false;
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
        case 'broadcast':
          ui.openBroadcast();
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
          if (keyContext.terminalFocused && keyContext.openFind) {
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
      { id: 'core.new-session', title: 'New Session', group: 'Sessions', shortcut: '⌘T', keywords: 'spawn agent terminal claude codex shell', run: () => (ui.newSessionOpen = true) },
      { id: 'core.ask-otto', title: 'Ask Otto (plain English)', group: 'Sessions', shortcut: '⌘I', keywords: 'orchestrate natural language command free text', run: () => ui.openPalette('english') },
      { id: 'core.broadcast', title: 'Broadcast message to sessions', group: 'Sessions', shortcut: '⌘⇧B', keywords: 'send message every agent tell all selected', run: () => ui.openBroadcast() },
      { id: 'core.close-tab', title: 'Close Tab', group: 'Sessions', shortcut: '⌘W', run: () => ws.closeActiveTab() },
      { id: 'core.next-session', title: 'Next Session', group: 'Sessions', shortcut: '⌘]', keywords: 'switch tab forward cycle', run: () => ws.cycleTab(1) },
      { id: 'core.prev-session', title: 'Previous Session', group: 'Sessions', shortcut: '⌘[', keywords: 'switch tab back cycle', run: () => ws.cycleTab(-1) },
      { id: 'core.split-v', title: 'Split Vertically', group: 'Sessions', shortcut: '⌘D', run: () => ws.split('col') },
      { id: 'core.split-h', title: 'Split Horizontally', group: 'Sessions', shortcut: '⌘⇧D', run: () => ws.split('row') },
      { id: 'core.new-workspace', title: 'Add Workspace', group: 'Workspaces', keywords: 'create new project folder directory', run: () => (ui.newWorkspaceOpen = true) },
      { id: 'core.update-clis', title: 'Update all CLIs', group: 'Sessions', shortcut: '⌘U', keywords: 'upgrade claude codex agy cli version', run: () => void updateAllCLIs() },
      { id: 'core.go-agents', title: 'Go to Agents', group: 'Navigate', keywords: 'module terminal', run: () => router.go('agents') },
      { id: 'core.go-connections', title: 'Go to Connections', group: 'Navigate', keywords: 'module ssh mysql redis', run: () => router.go('connections') },
      { id: 'core.go-git', title: 'Go to Git', group: 'Navigate', keywords: 'module repos prs pull requests', run: () => router.go('git') },
      { id: 'core.go-api', title: 'Go to API Client', group: 'Navigate', keywords: 'module postman http request rest curl', run: () => router.go('api') },
      { id: 'core.go-skills-eval', title: 'Go to Skills Evaluator', group: 'Navigate', keywords: 'module skill evaluate validate iterate improve', run: () => router.go('skills-eval') },
      { id: 'core.go-usage', title: 'Go to Usage & Metrics', group: 'Navigate', keywords: 'module usage cost tokens clickhouse metrics cpu ram billing analytics', run: () => router.go('usage') },
      { id: 'core.go-settings', title: 'Open Settings', group: 'Navigate', keywords: 'preferences appearance', run: () => router.go('settings/appearance') },
      { id: 'core.go-walkthroughs', title: 'Walkthroughs', group: 'Navigate', keywords: 'help intro tour videos onboarding', run: () => router.go('walkthroughs') },
      { id: 'core.go-tokens', title: 'Personal Access Tokens', group: 'Account', keywords: 'api token pat key secret cli script', run: () => router.go('settings/tokens') },
      { id: 'core.go-product', title: 'Go to Product', group: 'Navigate', keywords: 'product story jira confluence analysis rfc', run: () => router.go('product') },
      { id: 'core.go-insights', title: 'Go to Insights', group: 'Navigate', keywords: 'insights reports daily weekly monthly summary analytics activity', run: () => router.go('insights') },
      { id: 'core.go-swarm', title: 'Go to Swarm', group: 'Navigate', keywords: 'swarm agents team org orchestrator kanban board company', run: () => router.go('swarm') },
      { id: 'core.go-brokers', title: 'Go to Message Brokers', group: 'Navigate', keywords: 'message broker kafka redpanda topic consumer producer partition schema registry avro protobuf', run: () => router.go('brokers') },
      { id: 'core.toggle-rail', title: 'Toggle Sidebar', group: 'View', shortcut: '⌘1', run: () => ui.toggleRail() },
      { id: 'core.toggle-right', title: 'Toggle Right Panel', group: 'View', shortcut: '⌘J', run: () => ui.toggleRight() },
      { id: 'core.theme-native', title: 'Theme: Native', group: 'Appearance', run: () => ui.setTheme('native') },
      { id: 'core.theme-pro-dark', title: 'Theme: Pro Dark', group: 'Appearance', run: () => ui.setTheme('pro-dark') },
      { id: 'core.theme-warm', title: 'Theme: Warm', group: 'Appearance', run: () => ui.setTheme('warm') },
      { id: 'core.notes', title: 'Open Notes Panel', group: 'View', run: () => ui.openRight('notes') },
      { id: 'core.git-panel', title: 'Open Git Panel', group: 'View', run: () => ui.openRight('git') },
      { id: 'core.shortcuts', title: 'Keyboard Shortcuts', group: 'Help', shortcut: '?', keywords: 'keys cheat sheet bindings hotkeys', run: () => (shortcutsOpen = true) },
      { id: 'core.logout', title: 'Sign Out', group: 'Account', run: () => auth.logout() },
    ]);
    return unreg;
  });

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
      { id: 'focus.restart', title: 'Restart Focused Session', group: 'Session', keywords: 'reload reboot relaunch active current', run: () => void ws.restartSession(active.id) },
      { id: 'focus.archive', title: 'Archive Focused Session', group: 'Session', keywords: 'close hide stash active current', run: () => void ws.archiveSession(active.id) },
      { id: 'focus.rename', title: 'Rename Focused Session', group: 'Session', keywords: 'title name active current', run: () => void renameActiveSession() },
      ...(isAgent
        ? [{ id: 'focus.handover', title: 'Hand Over Focused Session…', group: 'Session', keywords: 'handoff transfer pass context active current', run: () => openSessionAction('handover') }]
        : []),
      { id: 'focus.attach-issue', title: 'Attach Jira Issue to Focused Session…', group: 'Session', keywords: 'jira ticket link story active current', run: () => openSessionAction('attach-issue') },
      { id: 'focus.attach-product', title: 'Attach Product Story to Focused Session…', group: 'Session', keywords: 'product story link context active current', run: () => openSessionAction('attach-product') },
    ];
    return registry.register('focused-session', cmds);
  });

  // ---- palette commands: workspaces ----
  $effect(() => {
    const unreg = registry.register(
      'workspaces',
      ws.workspaces.map((w) => ({
        id: `ws.${w.id}`,
        title: `Switch Workspace: ${w.name}`,
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
    const unreg = registry.register(
      'sessions',
      ws.sessions.map((s) => ({
        id: `session.${s.id}`,
        title: `Focus Session: ${s.title}`,
        group: 'Sessions',
        keywords: s.provider,
        run: () => {
          ws.navigateToSession(s.id);
        },
      })),
    );
    return unreg;
  });

  // ---- palette commands: connections ("connect <name>") ----
  $effect(() => {
    const wsId = ws.currentId;
    if (!wsId) return;
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
</script>

{#if router.module === 's'}
  <!-- Guest share view: full-screen terminal, no shell chrome. -->
  <SharePage sessionId={router.parts[1] ?? ''} />
{:else}
<!-- Center column: banners + notification bell + (agents) TabBar + the module
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
      <span>
        ⚠ A remote git provider returned <strong>502 Bad Gateway</strong> — it may be down or
        under maintenance. Your local work is unaffected; retries will resume automatically.
      </span>
      <span class="grow"></span>
      <button class="pb-dismiss" onclick={() => serviceHealth.dismiss()} aria-label="Dismiss notice">✕</button>
    </div>
  {/if}
  <div class="bell-anchor" class:tauri-top={isTauri}>
    <NotificationBell />
  </div>
  {#if moduleName === 'agents'}
    <TabBar bellGutter />
  {/if}
  <div class="content" class:bell-gutter={moduleName !== 'agents'}>
    {#if moduleName === 'agents'}
      <AgentsPage />
    {:else if moduleName === 'connections'}
      <ConnectionsPage />
    {:else if moduleName === 'git'}
      <GitPage />
    {:else if moduleName === 'api'}
      <ApiPage />
    {:else if moduleName === 'database'}
      <DatabasePage />
    {:else if moduleName === 'brokers'}
      <BrokersPage />
    {:else if moduleName === 'workflows'}
      <WorkflowsPage />
    {:else if moduleName === 'skills-eval'}
      <SkillsEvalPage />
    {:else if moduleName === 'usage'}
      <UsagePage />
    {:else if moduleName === 'settings'}
      <Settings />
    {:else if moduleName === 'walkthroughs'}
      <Walkthroughs />
    {:else if moduleName === 'product'}
      <ProductPage />
    {:else if moduleName === 'insights'}
      <InsightsPage />
    {:else if moduleName === 'swarm'}
      <SwarmPage />
    {:else if moduleName === 'vault'}
      <VaultPage />
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

{#if viewport.isDesktop}
<!-- DESKTOP (≥1025px): the original, unchanged 3-pane shell. -->
<!-- App zoom: Tauri uses the native WKWebView page-zoom (applyNativeZoom). In a
     BROWSER we used to apply CSS `zoom:${ui.zoom}` here, but CSS zoom (a) stretches
     the WebGL terminal canvas (oversized + clipped fit) and (b) breaks click
     hit-testing + absolutely-positioned popover/dropdown coordinates. So in the
     browser we DON'T CSS-zoom — users scale crisply with the browser's own zoom
     (⌘+/−), which re-rasterizes everything (terminal included) and keeps
     coordinates correct. ui.zoom still drives native zoom inside Tauri. -->
<div class="shell">
  <div class="shell-main">
    <div class="sidebar" class:tauri-top={isTauri}>
      {#if ui.railExpanded}
        <Navigator />
      {:else}
        <Rail />
      {/if}
    </div>

    <div class="center">
      {@render centerContent()}
    </div>

    <!-- Right panel (Activity/Git/Files/…) for the focused session. Shown in
         every Agents layout — tabbed, split, AND tiled — so per-session activity
         stays visible in multi-session views (it tracks `ws.activeSession`, the
         focused pane/tile), not just when a single session is on screen. -->
    {#if showRightPanel}
      <RightPanel />
    {/if}
  </div>

  <StatusBar />
</div>
{:else}
<!-- MOBILE (phone ≤640px / tablet 641–1024px): single-pane content with the
     Navigator and RightPanel moved off-canvas into drawers, a compact top bar,
     and (phone only) a bottom nav. Tablet keeps a persistent narrow Navigator
     column instead of the left drawer. -->
<div class="shell mobile" class:tablet={viewport.isTablet}>
  <header class="mtopbar" class:tauri-top={isTauri}>
    {#if viewport.isPhone}
      <button
        class="mtop-btn"
        onclick={() => ui.toggleRail()}
        title="Menu"
        aria-label="Open navigator"
      >
        <Icon name="sidebar" size={18} />
      </button>
    {/if}
    <!-- Back/Forward: visible whenever there is history to walk. Placed left of
         the title so the thumb can reach them comfortably on phone + tablet. -->
    <NavButtons />
    <span class="mtop-title">{moduleName === 'agents' ? (ws.activeSession?.title ?? 'Agents') : moduleName}</span>
    <span class="grow"></span>
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
       keyboard map calls; desktop is completely unaffected. -->
  {#if viewport.isPhone}
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

  <StatusBar />
</div>

<!-- Phone: Navigator lives in a LEFT drawer (reuses ui.railExpanded as its
     open-state). On tablet the Navigator is persistent, so no left drawer. -->
{#if viewport.isPhone}
  <Drawer bind:open={ui.railExpanded} side="left" label="Navigator" width="min(86vw, 280px)">
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
  <NewSession onclose={() => (ui.newSessionOpen = false)} />
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
    background: var(--bg);
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
  }
  .sidebar.tauri-top {
    /* room for overlaid traffic lights in the Tauri window */
    padding-top: 26px;
  }
  .center {
    flex: 1;
    min-width: 0;
    display: flex;
    flex-direction: column;
    position: relative;
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
    color: var(--accent);
    background: color-mix(in srgb, var(--accent) 16%, transparent);
  }
  .mtop-title {
    font-size: 13px;
    font-weight: 600;
    color: var(--text);
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    text-transform: capitalize;
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
    font-size: 12.5px;
    background: color-mix(in srgb, #e0a000 18%, var(--surface));
    color: var(--text);
    border-bottom: 1px solid color-mix(in srgb, #e0a000 45%, transparent);
    z-index: 5;
  }
  .pb-dismiss {
    flex-shrink: 0;
    border: none;
    background: transparent;
    color: var(--text-dim);
    cursor: pointer;
    font-size: 13px;
    padding: 2px 6px;
    border-radius: 4px;
    line-height: 1;
  }
  .pb-dismiss:hover {
    color: var(--text);
    background: color-mix(in srgb, var(--text) 10%, transparent);
  }
  /* Impersonation banner — blue tint to visually differentiate from the
     amber provider-health banner; Stop button is non-destructive styling. */
  .impersonation-banner {
    background: color-mix(in srgb, #3b82f6 18%, var(--surface));
    border-bottom-color: color-mix(in srgb, #3b82f6 45%, transparent);
  }
  .imp-real {
    font-size: 11.5px;
    color: var(--text-dim);
    margin-inline-start: 4px;
  }
  .imp-countdown {
    font-size: 11.5px;
    color: var(--text-dim);
    font-variant-numeric: tabular-nums;
    flex-shrink: 0;
  }
  .imp-countdown.imp-urgent {
    color: #ef4444;
    font-weight: 600;
  }
  /* Always-visible notification bell, anchored to the top-right of the main
     column so it's reachable from every module (the tab bar only renders on
     Agents). Sits above content; the dropdown opens downward from here. */
  .bell-anchor {
    position: absolute;
    top: 6px;
    inset-inline-end: 10px;
    z-index: 50;
  }
  .bell-anchor.tauri-top {
    top: 8px;
  }
  .content {
    flex: 1;
    min-height: 0;
    overflow: hidden;
  }
  /* Reserve room on the right of a module page so its header action buttons
     never sit under the floating notification bell. */
  .content.bell-gutter {
    padding-inline-end: 42px;
  }
</style>
