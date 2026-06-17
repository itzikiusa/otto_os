<script lang="ts">
  // Main shell (post-auth): rail/navigator + tab bar + module router +
  // right panel + status bar + palette + global keys.
  import Rail from './Rail.svelte';
  import Navigator from './Navigator.svelte';
  import TabBar from './TabBar.svelte';
  import RightPanel from './RightPanel.svelte';
  import StatusBar from './StatusBar.svelte';
  import Palette from './Palette.svelte';
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
  import WorkflowsPage from '../modules/workflows/WorkflowsPage.svelte';
  import SkillsEvalPage from '../modules/skills-eval/SkillsEvalPage.svelte';
  import UsagePage from '../modules/usage/UsagePage.svelte';
  import Settings from '../modules/settings/Settings.svelte';
  import Walkthroughs from '../modules/help/Walkthroughs.svelte';
  import { router } from '../lib/router.svelte';
  import { ui, isTauri } from '../lib/stores/ui.svelte';
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

  const moduleName = $derived(router.module === '' ? 'agents' : router.module);

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
          ui.openPalette('english', 'broadcast ');
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
      router.go('agents');
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
      { id: 'core.broadcast', title: 'Broadcast to all sessions', group: 'Sessions', shortcut: '⌘⇧B', keywords: 'send message every agent tell', run: () => ui.openPalette('english', 'broadcast ') },
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
      { id: 'core.toggle-rail', title: 'Toggle Sidebar', group: 'View', shortcut: '⌘1', run: () => ui.toggleRail() },
      { id: 'core.toggle-right', title: 'Toggle Right Panel', group: 'View', shortcut: '⌘J', run: () => ui.toggleRight() },
      { id: 'core.theme-native', title: 'Theme: Native', group: 'Appearance', run: () => ui.setTheme('native') },
      { id: 'core.theme-pro-dark', title: 'Theme: Pro Dark', group: 'Appearance', run: () => ui.setTheme('pro-dark') },
      { id: 'core.theme-warm', title: 'Theme: Warm', group: 'Appearance', run: () => ui.setTheme('warm') },
      { id: 'core.notes', title: 'Open Notes Panel', group: 'View', run: () => ui.openRight('notes') },
      { id: 'core.git-panel', title: 'Open Git Panel', group: 'View', run: () => ui.openRight('git') },
      { id: 'core.logout', title: 'Sign Out', group: 'Account', run: () => auth.logout() },
    ]);
    return unreg;
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
          ws.openSession(s.id);
          router.go('agents');
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
              router.go('agents');
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

<div class="shell" style={isTauri ? undefined : `zoom:${ui.zoom}`}>
  <div class="shell-main">
    <div class="sidebar" class:tauri-top={isTauri}>
      {#if ui.railExpanded}
        <Navigator />
      {:else}
        <Rail />
      {/if}
    </div>

    <div class="center">
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
        {:else}
          <AgentsPage />
        {/if}
      </div>
    </div>

    {#if moduleName === 'agents' && ws.singleSessionView}
      <RightPanel />
    {/if}
  </div>

  <StatusBar />
</div>

<Palette />

{#if ui.newSessionOpen}
  <NewSession onclose={() => (ui.newSessionOpen = false)} />
{/if}

{#if ui.newWorkspaceOpen}
  <NewWorkspace onclose={() => (ui.newWorkspaceOpen = false)} />
{/if}

<ConfirmDialog />
<ContextMenu />
<FindInPage />

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
  /* Always-visible notification bell, anchored to the top-right of the main
     column so it's reachable from every module (the tab bar only renders on
     Agents). Sits above content; the dropdown opens downward from here. */
  .bell-anchor {
    position: absolute;
    top: 6px;
    right: 10px;
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
    padding-right: 42px;
  }
</style>
