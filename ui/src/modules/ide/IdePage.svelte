<script lang="ts">
  import { onDestroy, untrack } from 'svelte';
  import { registry } from '../../lib/commands.svelte';
  import Badge from '../../lib/components/Badge.svelte';
  import EmptyState from '../../lib/components/EmptyState.svelte';
  import Icon from '../../lib/components/Icon.svelte';
  import IdeModeTabs from '../../lib/components/IdeModeTabs.svelte';
  import LazyMount from '../../lib/components/LazyMount.svelte';
  import PageBody from '../../lib/components/PageBody.svelte';
  import PageHeader from '../../lib/components/PageHeader.svelte';
  import PaneDivider from '../../lib/components/PaneDivider.svelte';
  import { confirmer } from '../../lib/confirm.svelte';
  import { keyContext } from '../../lib/keys';
  import { lazyComponent } from '../../lib/lazy-component.svelte';
  import { loadPaneWidth } from '../../lib/paneResizer';
  import { plural } from '../../lib/plural';
  import { router } from '../../lib/router.svelte';
  import { ui } from '../../lib/stores/ui.svelte';
  import { ws } from '../../lib/stores/workspace.svelte';
  import { idePreferences } from '../../lib/idePreferences.svelte';
  import { ideWorkspacePort } from '../../lib/ideWorkspacePort.svelte';
  import type { IdeCompareMode } from './history/ideComparisonLoader';
  import IdeEditorArea from './IdeEditorArea.svelte';
  import { ide } from './ideStore.svelte';
  import IdeWorkspaceSwitcher from './shell/IdeWorkspaceSwitcher.svelte';
  import IdeHeaderActions from './shell/IdeHeaderActions.svelte';
  import IdeStackSwitcher from './shell/IdeStackSwitcher.svelte';
  import { ideLayout, paneAfterFileOpen, STACK_MAX_H, type IdeStackPane } from './shell/ideLayout.svelte';
  import {
    handleIdeChord,
    handleIdeWindowKeyDown,
    promptIdeGoToLine,
    readQuickSchemePrefs,
  } from './shell/ideShellCommands';
  import { ideAgent } from './store/ideAgentController.svelte';

  const loadShellMenus = () => import('./shell/ideShellMenus');

  const IdeProjectTreeLazy = lazyComponent(() => import('./IdeProjectTree.svelte'));
  const FolderPickerLazy = lazyComponent(() => import('../../lib/components/FolderPicker.svelte'));
  const IdeBottomDockLazy = lazyComponent(() => import('./IdeBottomDock.svelte'));
  const IdeAgentPanelLazy = lazyComponent(() => import('./IdeAgentPanel.svelte'));
  const IdeBuildToolWindowLazy = lazyComponent(() => import('./buildtools/IdeBuildToolWindow.svelte'));
  const IdeSettingsModalLazy = lazyComponent(() => import('./settings/IdeSettingsModal.svelte'));
  const IdeBulkRepoPickerLazy = lazyComponent(() => import('./shell/IdeBulkRepoPickerModal.svelte'));
  const IdeSearchEverywhereLazy = lazyComponent(() => import('./IdeSearchEverywhere.svelte'));
  const IdeToolchainsModalLazy = lazyComponent(() => import('./IdeToolchainsModal.svelte'));
  const IdeEnvironmentsModalLazy = lazyComponent(() => import('./IdeEnvironmentsModal.svelte'));
  const IdeSshTargetModalLazy = lazyComponent(() => import('./IdeSshTargetModal.svelte'));
  const IdeShareModalLazy = lazyComponent(() => import('./IdeShareModal.svelte'));
  const IdePluginsModalLazy = lazyComponent(() => import('./IdePluginsModal.svelte'));
  const IdeComparisonModalLazy = lazyComponent(() => import('./history/IdeComparisonModal.svelte'));

  const TREE_W_KEY = 'otto.ide.treeW';
  const BUILD_TOOL_W_KEY = 'otto.ide.buildToolW';
  const AGENT_W_KEY = 'otto.ide.agentW';

  let quickPrefs = $state(readQuickSchemePrefs());
  let treeW = $state(loadPaneWidth(TREE_W_KEY, 270, 180, 620));
  let buildToolW = $state(loadPaneWidth(BUILD_TOOL_W_KEY, 310, 240, 720));
  let agentW = $state(loadPaneWidth(AGENT_W_KEY, 360, 260, 760));
  let explorerVisible = $state(true);
  // Stacked layout (phone, phone on its side, a narrow window): one pane at a
  // time, decided by this page's own size (shell/ideLayout.svelte.ts).
  const stack = $derived(ideLayout.stack);
  const pane = $derived(ideLayout.pane);
  // Workbench below 1000 px wide: at most ONE side panel next to the editor —
  // an open agent / build-tool dock hides the explorer rail instead of
  // squeezing the editor to a sliver.
  const treeShown = $derived(
    stack ? pane === 'files' : explorerVisible && !(ideLayout.width < 1000 && (ideAgent.panelOpen || ide.buildToolOpen)),
  );
  const agentShown = $derived(stack ? pane === 'agent' : ideAgent.panelOpen);
  // Typing in a stacked editor squeezed by the on-screen keyboard: the page
  // header steps aside so the code keeps its lines (class:typing below).
  let typing = $state(false);
  let folderPickerMode = $state<'open' | 'attach' | null>(null);
  let compareTarget = $state<{
    mode: IdeCompareMode;
    root: string;
    targetPath: string;
    secondPath: string | null;
    isDir: boolean;
    initialFilter: 'all' | 'deleted';
    initialRef: string | null;
    initialClipboardText?: string;
  } | null>(null);

  function resolveRootForPath(p: string): string {
    const clean = p.replace(/\/+$/, '') || '/';
    let best = '';
    for (let i = 0; i < ide.projectRoots.length; i++) {
      const rp = ide.projectRoots[i].path.replace(/\/+$/, '') || '/';
      if ((clean === rp || clean.startsWith(`${rp}/`)) && rp.length > best.length) best = rp;
    }
    return best || (ide.target.root.replace(/\/+$/, '') || '/');
  }

  function openCompareModal(opts: {
    mode: IdeCompareMode;
    path?: string;
    secondPath?: string | null;
    isDir?: boolean;
    filter?: 'all' | 'deleted';
    ref?: string | null;
    clipboardText?: string;
  }): void {
    const p = opts.path || ide.activePath || ide.target.root;
    if (!p) return;
    compareTarget = {
      mode: opts.mode,
      root: resolveRootForPath(p),
      targetPath: p,
      secondPath: opts.secondPath ?? null,
      isDir: opts.isDir ?? false,
      initialFilter: opts.filter ?? 'all',
      initialRef: opts.ref ?? null,
      initialClipboardText: opts.clipboardText,
    };
  }

  $effect(() => {
    if (typeof window === 'undefined') return;
    const onOpenSettings = (ev: Event): void => ide.openSettings((ev as CustomEvent<{ section?: string }>).detail?.section);
    const onPrefsChanged = (): void => { quickPrefs = readQuickSchemePrefs(); };
    const onPageHide = (): void => ide.suspendOnPageLeave();
    const onWindowBlur = (): void => {
      if (idePreferences.current.autoSave) void ide.flushAutoSave();
    };
    const onVisibilityChange = (): void => {
      if (document.hidden && idePreferences.current.autoSave) void ide.flushAutoSave();
    };
    const onLocalHistory = (ev: Event): void => {
      const d = (ev as CustomEvent<{ path?: string; isDir?: boolean; filter?: 'all' | 'deleted'; putLabel?: boolean } | undefined>).detail;
      const p = d?.path || ide.activePath || ide.target.root;
      if (!p) return;
      if (d?.putLabel) {
        void import('./history/ideComparisonLoader').then((m) => m.promptPutLocalHistoryLabel(resolveRootForPath(p), p));
        return;
      }
      openCompareModal({ mode: 'local-history', path: p, isDir: d?.isDir ?? p === ide.target.root, filter: d?.filter ?? 'all' });
    };
    const onCompare = (ev: Event): void => {
      const d = (ev as CustomEvent<{ mode?: IdeCompareMode; path?: string; secondPath?: string; isDir?: boolean; filter?: 'all' | 'deleted'; ref?: string; clipboardText?: string } | undefined>).detail;
      openCompareModal({ mode: d?.mode ?? 'local-history', path: d?.path, secondPath: d?.secondPath, isDir: d?.isDir, filter: d?.filter, ref: d?.ref, clipboardText: d?.clipboardText });
    };
    const onGitAction = (ev: Event): void => {
      const detail = (ev as CustomEvent<{ action?: string; path?: string } | undefined>).detail;
      void import('./git/ideGitQuickMenu').then((m) => m.runIdeGitActionEvent(detail, (mode, p) => openCompareModal({ mode, path: p })));
    };
    window.addEventListener('otto:ide-open-settings', onOpenSettings);
    window.addEventListener('otto:ide-prefs-changed', onPrefsChanged);
    window.addEventListener('otto:ide-open-local-history', onLocalHistory);
    window.addEventListener('otto:ide-open-compare', onCompare);
    window.addEventListener('otto:ide-git-action', onGitAction);
    window.addEventListener('blur', onWindowBlur);
    document.addEventListener('visibilitychange', onVisibilityChange);
    window.addEventListener('pagehide', onPageHide);
    window.addEventListener('beforeunload', onPageHide);
    return () => {
      window.removeEventListener('otto:ide-open-settings', onOpenSettings);
      window.removeEventListener('otto:ide-prefs-changed', onPrefsChanged);
      window.removeEventListener('otto:ide-open-local-history', onLocalHistory);
      window.removeEventListener('otto:ide-open-compare', onCompare);
      window.removeEventListener('otto:ide-git-action', onGitAction);
      window.removeEventListener('blur', onWindowBlur);
      document.removeEventListener('visibilitychange', onVisibilityChange);
      window.removeEventListener('pagehide', onPageHide);
      window.removeEventListener('beforeunload', onPageHide);
    };
  });

  function focusAgentComposer(tries = 15): void {
    IdeAgentPanelLazy.prefetch();
    requestAnimationFrame(() => {
      const el = document.querySelector<HTMLElement>('[data-testid="ide-agent-composer"]');
      if (el) el.focus();
      else if (tries > 0 && ideAgent.panelOpen) focusAgentComposer(tries - 1);
    });
  }

  function triggerAgentShortcut(): void {
    const hasSel = ide.cursorInfo.selections.some((s) => s.anchor !== s.head);
    if (!ideAgent.panelOpen && hasSel) {
      ideAgent.markSelectionFromEditor({ openPanel: true });
      focusAgentComposer();
      return;
    }
    ideAgent.togglePanel();
    if (ideAgent.panelOpen) focusAgentComposer();
  }

  let lastAgentOpen = false;
  $effect(() => {
    const open = ideAgent.panelOpen;
    ideWorkspacePort.setAgentDockOpen(open);
    if (open !== lastAgentOpen) {
      lastAgentOpen = open;
      untrack(() => {
        if (!ideLayout.stack) return;
        if (open) ideLayout.pane = 'agent';
        else if (ideLayout.pane === 'agent') ideLayout.pane = 'editor';
      });
    }
  });

  // Entering the stack remembers the workbench's agent dock; leaving it puts
  // that back, so a phone pane never opens (or closes) a desktop dock.
  let deskAgentOpen: boolean | null = null;
  $effect(() => {
    const isStack = stack;
    untrack(() => {
      if (isStack && deskAgentOpen === null) {
        deskAgentOpen = ideAgent.panelOpen;
        ideAgent.togglePanel(ideLayout.pane === 'agent');
      } else if (!isStack && deskAgentOpen !== null) {
        const restore = deskAgentOpen;
        deskAgentOpen = null;
        ideAgent.togglePanel(restore);
      }
    });
  });

  function showPane(next: IdeStackPane): void {
    ideLayout.pane = next;
    // The agent panel's subscriptions follow its open flag.
    if ((next === 'agent') !== ideAgent.panelOpen) ideAgent.togglePanel(next === 'agent');
  }

  // A file opened from the Files pane (a tap in the tree) shows the editor.
  let lastActivePath = ide.activePath;
  $effect(() => {
    const p = ide.activePath;
    if (p === lastActivePath) return;
    lastActivePath = p;
    untrack(() => {
      if (p && ideLayout.stack) ideLayout.pane = paneAfterFileOpen(ideLayout.pane);
    });
  });

  $effect(() => {
    void ws.currentId;
    void ws.workspaces.length;
    void ws.listSettled;
    untrack(() => ide.attachDefaultWorkspace());
  });

  let lastHandledQuery = '';
  $effect(() => {
    void router.parts;
    if (typeof window === 'undefined') return;
    const h = window.location.hash;
    const qIdx = h.indexOf('?');
    if (qIdx < 0) { lastHandledQuery = ''; return; }
    const rawQuery = h.slice(qIdx + 1);
    if (!rawQuery || rawQuery === lastHandledQuery) return;
    lastHandledQuery = rawQuery;
    const params = new URLSearchParams(rawQuery);
    const collabId = params.get('collab');
    const token = params.get('token');
    const guestName = params.get('name') || 'Teammate';
    const projectParam = params.get('project');
    const rootParam = params.get('root');
    const fileParam = params.get('file');
    const lineParam = Number(params.get('line') || '0') || undefined;

    untrack(() => {
      if (collabId && token) {
        history.replaceState(null, '', '#/ide');
        lastHandledQuery = '';
        void ide.joinCollabRoom(collabId, token, guestName);
        return;
      }
      void (async () => {
        try {
          if (projectParam) await ide.activateProject(projectParam);
          // A root already attached to the open (multi-folder) project keeps that project and its tabs.
          else if (rootParam && !(ide.target.kind === 'local' && ide.projectRoots.some((r) => r.path.replace(/\/+$/, '') === rootParam.replace(/\/+$/, '')))) {
            if (!(await ide.setLocalTarget(rootParam, null, { skipAutoOpen: Boolean(fileParam) }))) return;
          }
          else ide.attachDefaultWorkspace();
          if (fileParam) {
            await ide.openFile(fileParam, { line: lineParam, preview: false });
            ideLayout.pane = 'editor';
          }
        } finally {
          if (window.location.hash.endsWith(`?${rawQuery}`)) {
            history.replaceState(null, '', '#/ide');
            lastHandledQuery = '';
          }
        }
      })();
    });
  });

  $effect(() => {
    let cleanup: (() => void) | null = null;
    let disposed = false;
    void loadShellMenus().then((m) => {
      if (disposed) return;
      cleanup = registry.register(
        'ide',
        m.buildIdePaletteCommands({
          onAttachFolder: () => (folderPickerMode = 'attach'),
          onTriggerAgent: () => triggerAgentShortcut(),
        }),
      );
    });
    return () => {
      disposed = true;
      cleanup?.();
    };
  });

  $effect(() => {
    const claim = (e: KeyboardEvent): boolean =>
      handleIdeChord(e, ui.overlayOpen, () => (explorerVisible = !explorerVisible), () => triggerAgentShortcut());
    keyContext.pageChords = claim;
    return () => {
      if (keyContext.pageChords === claim) keyContext.pageChords = null;
    };
  });

  $effect(() => {
    let lastShiftUp = 0;
    let shiftDirty = false;
    const onKeyDown = (e: KeyboardEvent): void => {
      if (e.key === 'Shift') {
        if (!e.repeat) shiftDirty = Boolean(e.metaKey || e.ctrlKey || e.altKey);
        return;
      }
      shiftDirty = true;
      lastShiftUp = 0;
      handleIdeWindowKeyDown(e, ui.overlayOpen, () => triggerAgentShortcut());
    };
    const onKeyUp = (e: KeyboardEvent): void => {
      if (e.key !== 'Shift') return;
      if (shiftDirty || e.metaKey || e.ctrlKey || e.altKey) { shiftDirty = false; lastShiftUp = 0; return; }
      const now = Date.now();
      if (now - lastShiftUp < 380 && !ui.overlayOpen) {
        lastShiftUp = 0;
        ide.searchScopePath = null; ide.searchReplaceMode = false; ide.searchInitialTab = 'all'; ide.searchEverywhereOpen = true;
      } else {
        lastShiftUp = now;
      }
    };
    window.addEventListener('keydown', onKeyDown);
    window.addEventListener('keyup', onKeyUp);
    return () => {
      window.removeEventListener('keydown', onKeyDown);
      window.removeEventListener('keyup', onKeyUp);
    };
  });

  // Preserve CodeMirror `EditorState` & ⌘Z undo stack across screen switches.
  onDestroy(() => {
    ideWorkspacePort.setAgentDockOpen(false);
    ide.suspendOnPageLeave();
    ideAgent.dispose();
  });

  async function leaveOrEndCollab(): Promise<void> {
    if (ide.collabIsGuest) { ide.leaveCollabRoom(); return; }
    const ok = await confirmer.ask('Disconnect all participants and revoke the share link?', {
      title: 'Stop sharing session?', confirmLabel: 'Stop sharing', danger: true,
    });
    if (ok) await ide.stopCollabRoom();
  }

  async function handlePickedFolder(picked: string): Promise<void> {
    const mode = folderPickerMode;
    folderPickerMode = null;
    if (!picked) return;
    if (mode === 'attach' && ide.hasTarget) await ide.addProjectFolder(picked);
    else await ide.setLocalTarget(picked);
  }
</script>

<div
  class="ide-page"
  class:stack
  class:short-stack={stack && ideLayout.height > 0 && ideLayout.height < STACK_MAX_H}
  class:typing={stack && typing && ideLayout.height > 0 && ideLayout.height < 360}
  onfocusin={(e) => (typing = e.target instanceof Element && !!e.target.closest('.cm-editor'))}
  onfocusout={() => (typing = false)}
  data-pane={stack ? pane : undefined}
  data-testid="ide-page"
  data-ide-density={idePreferences.current.uiDensity}
  bind:clientWidth={ideLayout.width}
  bind:clientHeight={ideLayout.height}
>
  <!-- One toolbar row (G13). Stacked: the pane switcher takes the tabs slot
       (below the bar when the page is tall, inline when it is short). -->
  <PageHeader title={stack ? 'IDE' : 'IDE (Beta)'} tabsPlacement={stack && ideLayout.height >= STACK_MAX_H ? 'below' : 'inline'}>
    {#snippet tabs()}
      {#if stack && ide.hasTarget}<IdeStackSwitcher onshow={showPane} />{:else}<IdeModeTabs active="project" />{/if}
    {/snippet}

    {#snippet badge()}
      <IdeWorkspaceSwitcher onpickfolder={(m) => (folderPickerMode = m)} />
    {/snippet}

    {#snippet actions()}
      <IdeHeaderActions
        {stack}
        bind:quickPrefs
        {explorerVisible}
        onToggleExplorer={() => (explorerVisible = !explorerVisible)}
        onTriggerAgent={() => triggerAgentShortcut()}
        onAttachFolder={() => (folderPickerMode = 'attach')}
        onShowPane={showPane}
        onOpenScratches={() => router.go('workbench')}
        onToggleAgentDock={() => { ideAgent.togglePanel(); if (ideAgent.panelOpen) focusAgentComposer(); }}
      />
    {/snippet}
  </PageHeader>


  {#if ide.collabRoom}
    <div class="ide-collab-banner" role="region" aria-label="Live collaboration session">
      <Badge tone="ok" live label={ide.collabIsGuest ? 'Joined live session' : 'Sharing live'} />
      <span class="collab-title">{ide.collabRoom.title}</span>
      <span class="collab-count">{plural(ide.collabRoom.peers.length, 'participant')}</span>
      <div class="collab-peers">
        {#each ide.collabRoom.peers.filter((p) => p.peer_id !== ide.collabPeerId) as peer (peer.peer_id)}
          <button
            type="button"
            class="btn small ghost"
            aria-pressed={ide.followingPeerId === peer.peer_id}
            onclick={() => ide.followPeer(ide.followingPeerId === peer.peer_id ? null : peer.peer_id)}
            title={ide.followingPeerId === peer.peer_id ? `Stop following ${peer.display_name}` : `Follow ${peer.display_name}`}
          >
            <Icon name="user" size={12} />
            {ide.followingPeerId === peer.peer_id ? `Following ${peer.display_name}` : peer.display_name}
          </button>
        {/each}
      </div>
      <div class="collab-actions">
        {#if !ide.collabIsGuest}
          <button type="button" class="btn small ghost" onclick={() => (ide.shareOpen = true)}>Manage</button>
        {/if}
        <button type="button" class="btn small ghost" onclick={() => void leaveOrEndCollab()}>
          {ide.collabIsGuest ? 'Leave' : 'End…'}
        </button>
      </div>
    </div>
  {:else if ide.collabIsGuest && ide.collabError}
    <div class="ide-collab-banner" role="alert">
      <Badge tone="warn" label="Session ended" />
      <span class="collab-title">{ide.collabError}</span>
    </div>
  {/if}

  <PageBody width="full" padded={false} fill>
    {#if !ide.hasTarget}
      <EmptyState
        variant="page"
        headingLevel={2}
        icon="ide"
        title="Open a project in IDE"
        body="Edit multi-repo workspaces, manage Go & JDK toolchains, run Maven/Gradle lifecycles, set live DAP breakpoints, or code over SSH."
        actionLabel="Open local folder"
        actionIcon="folder"
        onaction={() => (folderPickerMode = 'open')}
      >
        <button type="button" class="btn ghost" onclick={() => (ide.bulkRepoPickerMode = 'create')}>
          <Icon name="layers" size={13} />
          Create multi-repo project…
        </button>
        <button type="button" class="btn ghost" onclick={() => ide.openSettings('build.ssh')}>
          <Icon name="server" size={13} />
          Connect over SSH…
        </button>
        <button type="button" class="btn ghost" onclick={() => router.go('workbench')} data-testid="ide-open-scratches">
          <Icon name="workbench" size={13} />
          Open Scratches
        </button>
      </EmptyState>
    {:else}
      <!-- ONE tree for every layout: the stack only hides panes, so rotating a
           phone (or resizing) never remounts the editor, terminal or agent. -->
      <div class="ide-workbench" class:stack>
        {#if treeShown}
          <div class="ide-rail-wrap" style:inline-size={stack ? undefined : `${treeW}px`}>
            <LazyMount lazy={IdeProjectTreeLazy} what="project explorer" variant="panel" props={{ onOpenCompare: openCompareModal }} />
          </div>
          {#if !stack}<PaneDivider bind:width={treeW} storageKey={TREE_W_KEY} min={180} max={620} defaultWidth={270} label="Resize project explorer" />{/if}
        {/if}

        <div class="ide-center-col" class:pane-off={stack && pane !== 'editor' && pane !== 'tools'} class:tools-only={stack && pane === 'tools'}>
          <IdeEditorArea />
          <LazyMount lazy={IdeBottomDockLazy} what="bottom tool dock" quiet props={{ stack, stackShowing: stack && pane === 'tools' }} />
        </div>

        {#if ide.buildToolOpen && !stack}
          <PaneDivider bind:width={buildToolW} storageKey={BUILD_TOOL_W_KEY} min={240} max={720} defaultWidth={310} invert label="Resize build tool window" />
          <div class="ide-bt-wrap" style:inline-size="{buildToolW}px">
            <LazyMount lazy={IdeBuildToolWindowLazy} what="build tool window" variant="panel" />
          </div>
        {/if}

        {#if agentShown}
          {#if !stack}<PaneDivider bind:width={agentW} storageKey={AGENT_W_KEY} min={260} max={760} defaultWidth={360} invert label="Resize AI agent dock" />{/if}
          <div class="ide-agent-wrap" style:inline-size={stack ? undefined : `${agentW}px`}>
            <LazyMount lazy={IdeAgentPanelLazy} what="AI agent" variant="panel" />
          </div>
        {/if}
      </div>
    {/if}
  </PageBody>

  {#if folderPickerMode}
    <LazyMount
      lazy={FolderPickerLazy}
      what="folder picker"
      quiet
      props={{
        title: folderPickerMode === 'attach' ? 'Add folder to project' : 'Open project folder',
        start: ide.target.kind === 'local' ? ide.target.root : '',
        onpick: (p: string) => void handlePickedFolder(p),
        onclose: () => (folderPickerMode = null),
      }}
      onerror={() => (folderPickerMode = null)}
    />
  {/if}

  {#if ide.settingsOpen}
    <LazyMount lazy={IdeSettingsModalLazy} what="IDE settings" quiet onerror={() => (ide.settingsOpen = false)} />
  {/if}
  {#if ide.bulkRepoPickerMode}
    <LazyMount
      lazy={IdeBulkRepoPickerLazy}
      props={{ mode: ide.bulkRepoPickerMode, onclose: () => (ide.bulkRepoPickerMode = null) }}
      what="multi-repo picker"
      quiet
      onerror={() => (ide.bulkRepoPickerMode = null)}
    />
  {/if}
  {#if ide.searchEverywhereOpen}
    <LazyMount lazy={IdeSearchEverywhereLazy} what="search everywhere" quiet onerror={() => (ide.searchEverywhereOpen = false)} />
  {/if}
  {#if ide.environmentsOpen}
    <LazyMount
      lazy={IdeEnvironmentsModalLazy}
      props={{ open: true, initialTab: ide.environmentsInitialTab, onclose: () => (ide.environmentsOpen = false) }}
      what="environments"
      quiet
      onerror={() => (ide.environmentsOpen = false)}
    />
  {/if}
  {#if ide.toolchainsOpen}
    <LazyMount
      lazy={IdeToolchainsModalLazy}
      props={{ open: true, onclose: () => (ide.toolchainsOpen = false) }}
      what="toolchains manager"
      quiet
      onerror={() => (ide.toolchainsOpen = false)}
    />
  {/if}
  {#if ide.sshOpen}
    <LazyMount
      lazy={IdeSshTargetModalLazy}
      props={{ open: true, onclose: () => (ide.sshOpen = false) }}
      what="SSH target picker"
      quiet
      onerror={() => (ide.sshOpen = false)}
    />
  {/if}
  {#if ide.shareOpen}
    <LazyMount
      lazy={IdeShareModalLazy}
      props={{ open: true, onclose: () => (ide.shareOpen = false) }}
      what="collaboration settings"
      quiet
      onerror={() => (ide.shareOpen = false)}
    />
  {/if}
  {#if ide.pluginsOpen}
    <LazyMount
      lazy={IdePluginsModalLazy}
      props={{ open: true, onclose: () => (ide.pluginsOpen = false) }}
      what="extensions manager"
      quiet
      onerror={() => (ide.pluginsOpen = false)}
    />
  {/if}
  {#if compareTarget}
    <LazyMount lazy={IdeComparisonModalLazy} what="comparison tool" quiet props={{ ...compareTarget, onclose: () => (compareTarget = null) }} onerror={() => (compareTarget = null)} />
  {/if}
</div>

<style>
  .ide-page { display: flex; flex-direction: column; flex: 1; min-inline-size: 0; min-block-size: 0; block-size: 100%; --ide-row-py: 4px; --ide-row-px: 8px; --ide-tab-h: 32px; --ide-bar-py: 6px; }
  .ide-page[data-ide-density='compact'] { --ide-row-py: 2px; --ide-row-px: 6px; --ide-tab-h: 28px; --ide-bar-py: 4px; }
  .ide-page[data-ide-density='comfortable'] { --ide-row-py: 6px; --ide-row-px: 10px; --ide-tab-h: 34px; --ide-bar-py: 8px; }
  .ide-collab-banner { display: flex; align-items: center; gap: 8px; padding-block: var(--ide-bar-py); padding-inline: 14px; background: var(--surface); border-block-end: 1px solid var(--border); font-size: var(--fs-xs); flex-wrap: wrap; }
  .collab-title { font-weight: 500; color: var(--text); }
  .collab-count { color: var(--text-dim); }
  .collab-peers { display: flex; align-items: center; gap: 4px; flex-wrap: wrap; }
  .collab-actions { display: flex; align-items: center; gap: 4px; margin-inline-start: auto; }
  .ide-workbench { display: flex; flex: 1; min-inline-size: 0; min-block-size: 0; }
  .ide-rail-wrap, .ide-bt-wrap, .ide-agent-wrap { flex-shrink: 0; min-inline-size: 0; min-block-size: 0; block-size: 100%; display: flex; flex-direction: column; }
  .ide-center-col { flex: 1; min-inline-size: 0; min-block-size: 0; display: flex; flex-direction: column; }
  /* Stacked: the visible pane fills the page; the others are hidden, not unmounted. */
  .ide-workbench.stack > .ide-rail-wrap, .ide-workbench.stack > .ide-agent-wrap { flex: 1; min-inline-size: 0; }
  .ide-center-col.pane-off { display: none; }
  .ide-center-col.tools-only :global(.ide-editor-area) { display: none; }
  /* Stacked chrome: no second breadcrumb under the code, no fold column; on a
     short page (phone on its side) no sub-bar. */
  /* CodeMirror's base theme declares `.cm-gutter { display: flex !important }`. */
  .ide-page.stack :global(.ide-breadcrumbs-bar) { display: none; }
  .ide-page.stack :global(.cm-gutter.cm-foldGutter) { display: none !important; }
  .ide-page.short-stack :global(.ide-subbar) { display: none; }
  .ide-page.typing :global(.page-header-bar) { display: none; }
  @media (max-width: 1024px) {
    .ide-workbench:not(.stack) > .ide-rail-wrap { max-inline-size: 32%; }
    .ide-bt-wrap, .ide-workbench:not(.stack) > .ide-agent-wrap { max-inline-size: 38%; }
  }
</style>
