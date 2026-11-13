<script lang="ts">
  import { onDestroy, untrack } from 'svelte';
  import { registry } from '../../lib/commands.svelte';
  import Badge from '../../lib/components/Badge.svelte';
  import EmptyState from '../../lib/components/EmptyState.svelte';
  import FolderPicker from '../../lib/components/FolderPicker.svelte';
  import Icon from '../../lib/components/Icon.svelte';
  import LazyMount from '../../lib/components/LazyMount.svelte';
  import PageBody from '../../lib/components/PageBody.svelte';
  import PageHeader from '../../lib/components/PageHeader.svelte';
  import PaneDivider from '../../lib/components/PaneDivider.svelte';
  import { confirmer } from '../../lib/confirm.svelte';
  import { ctxMenu, type MenuItem } from '../../lib/contextmenu.svelte';
  import { keyContext } from '../../lib/keys';
  import { lazyComponent } from '../../lib/lazy-component.svelte';
  import { LIST_PANE, loadPaneWidth } from '../../lib/paneResizer';
  import { plural } from '../../lib/plural';
  import { router } from '../../lib/router.svelte';
  import { onTabKey } from '../../lib/tabKeys';
  import { ui } from '../../lib/stores/ui.svelte';
  import { viewport } from '../../lib/stores/viewport.svelte';
  import { ws } from '../../lib/stores/workspace.svelte';
  import IdeBottomDock from './IdeBottomDock.svelte';
  import IdeEditorArea from './IdeEditorArea.svelte';
  import IdeProjectTree from './IdeProjectTree.svelte';
  import { ide, ideBasename } from './ideStore.svelte';
  import IdeRunToolbar from './shell/IdeRunToolbar.svelte';
  import IdeWorkspaceSwitcher from './shell/IdeWorkspaceSwitcher.svelte';
  import { ideAgent } from './store/ideAgentController.svelte';

  const IdeAgentPanelLazy = lazyComponent(() => import('./IdeAgentPanel.svelte'));
  const IdeSearchEverywhereLazy = lazyComponent(() => import('./IdeSearchEverywhere.svelte'));
  const IdeToolchainsModalLazy = lazyComponent(() => import('./IdeToolchainsModal.svelte'));
  const IdeEnvironmentsModalLazy = lazyComponent(() => import('./IdeEnvironmentsModal.svelte'));
  const IdeSshTargetModalLazy = lazyComponent(() => import('./IdeSshTargetModal.svelte'));
  const IdeShareModalLazy = lazyComponent(() => import('./IdeShareModal.svelte'));
  const IdePluginsModalLazy = lazyComponent(() => import('./IdePluginsModal.svelte'));

  const TREE_W_KEY = 'otto.ide.treeW';
  const AGENT_W_KEY = 'otto.ide.agentW';
  let treeW = $state(loadPaneWidth(TREE_W_KEY, 270, LIST_PANE.min, LIST_PANE.max));
  let agentW = $state(loadPaneWidth(AGENT_W_KEY, 360, 280, 560));
  let explorerVisible = $state(true);
  let mobileView = $state<'files' | 'editor' | 'tools' | 'agent'>('editor');
  let folderPickerMode = $state<'open' | 'attach' | null>(null);

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
    if (open !== lastAgentOpen) {
      lastAgentOpen = open;
      if (viewport.isPhone) {
        if (open) mobileView = 'agent';
        else if (mobileView === 'agent') mobileView = 'editor';
      }
    }
  });

  $effect(() => {
    void ws.currentId;
    void ws.workspaces.length;
    untrack(() => ide.attachDefaultWorkspace());
  });

  // Deep-link handling: `#/ide?file=...&root=...&line=...` or `#/ide?collab=...&token=...`.
  let lastHandledQuery = '';
  $effect(() => {
    void router.parts;
    if (typeof window === 'undefined') return;
    const h = window.location.hash;
    const qIdx = h.indexOf('?');
    if (qIdx < 0) return;
    const rawQuery = h.slice(qIdx + 1);
    if (!rawQuery || rawQuery === lastHandledQuery) return;
    lastHandledQuery = rawQuery;
    const params = new URLSearchParams(rawQuery);
    const collabId = params.get('collab');
    const token = params.get('token');
    const guestName = params.get('name') || 'Teammate';
    const rootParam = params.get('root');
    const fileParam = params.get('file');
    const lineParam = Number(params.get('line') || '0') || undefined;

    untrack(() => {
      if (collabId && token) {
        history.replaceState(null, '', '#/ide');
        void ide.joinCollabRoom(collabId, token, guestName);
        return;
      }
      void (async () => {
        if (rootParam) await ide.setLocalTarget(rootParam);
        else ide.attachDefaultWorkspace();
        if (fileParam) {
          await ide.openFile(fileParam, { line: lineParam, preview: false });
          mobileView = 'editor';
        }
      })();
    });
  });

  // Register ⌘K command palette commands while IDE is mounted.
  $effect(() =>
    registry.register('ide', [
      {
        id: 'ide.searchEverywhere',
        title: 'Search everywhere…',
        group: 'IDE',
        shortcut: '⇧⇧',
        keywords: 'find symbol file action replace',
        run: () => {
          ide.searchScopePath = null;
          ide.searchReplaceMode = false;
          ide.searchInitialTab = 'all';
          ide.searchEverywhereOpen = true;
        },
      },
      { id: 'ide.findInPath', title: 'Find in path…', group: 'IDE', shortcut: '⌘⇧F', run: () => ide.openFindInPath({ replace: false }) },
      { id: 'ide.replaceInPath', title: 'Replace in path…', group: 'IDE', shortcut: '⌘⇧R', run: () => ide.openFindInPath({ replace: true }) },
      { id: 'ide.run', title: 'Run configuration', group: 'IDE', shortcut: '⌃R', run: () => void ide.startRun() },
      { id: 'ide.debug', title: 'Debug configuration', group: 'IDE', shortcut: '⌃D', run: () => void ide.startDebug() },
      { id: 'ide.editConfigs', title: 'Edit run / debug configurations…', group: 'IDE', run: () => ide.openEnvironmentsModal('configs') },
      { id: 'ide.addFolder', title: 'Add folder to project…', group: 'IDE', run: () => (folderPickerMode = 'attach') },
      { id: 'ide.revealFile', title: 'Select opened file in project tree', group: 'IDE', run: () => void ide.revealInProjectTree() },
      { id: 'ide.toggleAgent', title: 'Toggle AI Agent', group: 'IDE', shortcut: '⌘L', keywords: 'ai assistant chat context', run: () => triggerAgentShortcut() },
      { id: 'ide.inlineEdit', title: 'Inline AI edit…', group: 'IDE', shortcut: '⌘I', keywords: 'ai refactor rewrite selection', run: () => ideAgent.openInlineEdit() },
      { id: 'ide.toolchains', title: 'Toolchains & SDKs…', group: 'IDE', run: () => (ide.toolchainsOpen = true) },
      { id: 'ide.environments', title: 'Environments & run configurations…', group: 'IDE', run: () => ide.openEnvironmentsModal('env') },
      { id: 'ide.ssh', title: 'Connect remote SSH project…', group: 'IDE', run: () => (ide.sshOpen = true) },
      { id: 'ide.share', title: 'Share live IDE session…', group: 'IDE', run: () => (ide.shareOpen = true) },
      { id: 'ide.plugins', title: 'Manage IDE plugins…', group: 'IDE', run: () => (ide.pluginsOpen = true) },
    ]),
  );

  const inXterm = (t: EventTarget | null): boolean =>
    t instanceof Element && Boolean(t.closest('.xterm'));

  // Claim ⌘-chords before the global shell keymap.
  $effect(() => {
    const claim = (e: KeyboardEvent): boolean => {
      if (ui.overlayOpen) return false;
      const mod = e.metaKey || e.ctrlKey;
      if (mod && !e.shiftKey && !e.altKey && e.code === 'KeyS') {
        e.preventDefault();
        void ide.saveFile();
        return true;
      }
      if (mod && !e.shiftKey && e.altKey && e.code === 'KeyS') {
        e.preventDefault();
        void ide.saveAll();
        return true;
      }
      if (mod && !e.shiftKey && !e.altKey && e.code === 'KeyE') {
        e.preventDefault();
        openRecentFilesMenu();
        return true;
      }
      if (mod && !e.shiftKey && !e.altKey && e.code === 'BracketLeft') {
        if (ide.canNavigateBack) {
          e.preventDefault();
          void ide.navigateBack();
          return true;
        }
      }
      if (mod && !e.shiftKey && !e.altKey && e.code === 'BracketRight') {
        if (ide.canNavigateForward) {
          e.preventDefault();
          void ide.navigateForward();
          return true;
        }
      }
      if (mod && e.shiftKey && !e.altKey && e.code === 'KeyF') {
        e.preventDefault();
        ide.openFindInPath({ replace: false });
        return true;
      }
      if (mod && e.shiftKey && !e.altKey && e.code === 'KeyR') {
        e.preventDefault();
        ide.openFindInPath({ replace: true });
        return true;
      }
      if (mod && e.shiftKey && !e.altKey && e.code === 'KeyO') {
        e.preventDefault();
        ide.searchScopePath = null;
        ide.searchReplaceMode = false;
        ide.searchInitialTab = 'files';
        ide.searchEverywhereOpen = true;
        return true;
      }
      if (mod && !e.shiftKey && !e.altKey && e.code === 'KeyO') {
        e.preventDefault();
        ide.searchScopePath = null;
        ide.searchReplaceMode = false;
        ide.searchInitialTab = 'symbols';
        ide.searchEverywhereOpen = true;
        return true;
      }
      if (mod && !e.shiftKey && !e.altKey && e.code === 'KeyI' && !(e.ctrlKey && !e.metaKey && inXterm(e.target))) {
        e.preventDefault();
        ideAgent.openInlineEdit();
        return true;
      }
      if (mod && e.altKey && !e.shiftKey && e.code === 'KeyL') {
        e.preventDefault();
        void ide.formatActiveFile();
        return true;
      }
      if (mod && !e.shiftKey && !e.altKey && e.key === 'F8') {
        if (ide.activePath) {
          e.preventDefault();
          ide.toggleBreakpoint(ide.activePath, ide.cursorInfo.line);
          return true;
        }
      }
      if (mod && !e.shiftKey && !e.altKey && e.key === 'F2') {
        e.preventDefault();
        void ide.stopActiveExecution();
        return true;
      }
      return false;
    };
    keyContext.pageChords = claim;
    return () => {
      if (keyContext.pageChords === claim) keyContext.pageChords = null;
    };
  });

  // Double-Shift (⇧⇧) Search Everywhere + AI Agent + debug/refactor shortcuts.
  $effect(() => {
    let lastShiftUp = 0;
    let shiftDirty = false;

    const onKeyDown = (e: KeyboardEvent): void => {
      if (e.key === 'Shift') {
        if (e.metaKey || e.ctrlKey || e.altKey) shiftDirty = true;
        return;
      }
      shiftDirty = true;
      if (ui.overlayOpen) return;

      if (!e.defaultPrevented && (e.metaKey || e.ctrlKey) && !e.shiftKey && !e.altKey && !(e.ctrlKey && !e.metaKey && inXterm(e.target))) {
        if (e.code === 'KeyL') {
          e.preventDefault();
          triggerAgentShortcut();
          return;
        }
        if (e.code === 'KeyI') {
          e.preventDefault();
          ideAgent.openInlineEdit();
          return;
        }
      }

      if (e.ctrlKey && !e.metaKey && !e.altKey && !e.shiftKey && e.code === 'KeyR') {
        e.preventDefault();
        void ide.startRun();
      } else if (e.ctrlKey && !e.metaKey && !e.altKey && !e.shiftKey && e.code === 'KeyD') {
        e.preventDefault();
        void ide.startDebug();
      } else if (e.shiftKey && !e.metaKey && !e.ctrlKey && !e.altKey && e.key === 'F6') {
        if (ide.activePath) {
          e.preventDefault();
          void ide.openRenameModal(ide.activePath, ide.cursorInfo.line, ide.cursorInfo.col);
        }
      } else if (!e.metaKey && !e.ctrlKey && !e.altKey && !e.shiftKey && e.key === 'F8') {
        if (ide.activeDebug?.state === 'paused') { e.preventDefault(); void ide.stepOver(); }
      } else if (!e.metaKey && !e.ctrlKey && !e.altKey && !e.shiftKey && e.key === 'F7') {
        if (ide.activeDebug?.state === 'paused') { e.preventDefault(); void ide.stepInto(); }
      } else if (e.shiftKey && !e.metaKey && !e.ctrlKey && !e.altKey && e.key === 'F8') {
        if (ide.activeDebug?.state === 'paused') { e.preventDefault(); void ide.stepOut(); }
      } else if (!e.metaKey && !e.ctrlKey && !e.altKey && !e.shiftKey && e.key === 'F9') {
        if (ide.activeDebug?.state === 'paused') { e.preventDefault(); void ide.resumeDebug(); }
      } else if (e.altKey && !e.metaKey && !e.ctrlKey && !e.shiftKey && e.key === 'F9') {
        if (ide.activeDebug?.state === 'paused') { e.preventDefault(); void ide.runToCursor(); }
      }
    };

    const onKeyUp = (e: KeyboardEvent): void => {
      if (e.key !== 'Shift') return;
      if (shiftDirty || e.metaKey || e.ctrlKey || e.altKey) {
        shiftDirty = false;
        lastShiftUp = 0;
        return;
      }
      const now = Date.now();
      if (now - lastShiftUp < 380 && !ui.overlayOpen) {
        lastShiftUp = 0;
        ide.searchScopePath = null;
        ide.searchReplaceMode = false;
        ide.searchInitialTab = 'all';
        ide.searchEverywhereOpen = true;
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

  onDestroy(() => {
    ide.disposeStreams();
    ideAgent.dispose();
  });

  function openRecentFilesMenu(): void {
    const anchor = document.querySelector<HTMLElement>('[data-testid="ide-more-btn"]');
    if (!anchor) return;
    const items: MenuItem[] =
      ide.recentPaths.length > 0
        ? ide.recentPaths.map((p) => ({
            label: ideBasename(p),
            hint: ide.relativePath(p),
            icon: 'file',
            action: () => void ide.openFile(p, { preview: false }),
          }))
        : [{ label: 'No recent files yet', disabled: true }];
    ctxMenu.showAt(anchor, items, { align: 'end' });
  }

  function openMoreMenu(e: MouseEvent): void {
    const items: MenuItem[] = [
      {
        label: 'Search everywhere…',
        icon: 'search',
        hint: '⇧⇧',
        action: () => {
          ide.searchScopePath = null;
          ide.searchReplaceMode = false;
          ide.searchInitialTab = 'all';
          ide.searchEverywhereOpen = true;
        },
      },
      {
        label: 'Find in path…',
        icon: 'search',
        hint: '⌘⇧F',
        disabled: !ide.hasTarget,
        action: () => ide.openFindInPath({ replace: false }),
      },
      {
        label: 'Multi-line edit (prefix / suffix)',
        icon: 'edit',
        checked: ide.prefixSuffixOpen,
        disabled: !ide.activeTab,
        action: () => (ide.prefixSuffixOpen = !ide.prefixSuffixOpen),
      },
      {
        label: 'Format on save',
        checked: ide.formatOnSave,
        action: () => ide.setFormatOnSave(!ide.formatOnSave),
      },
      { separator: true },
      {
        label: explorerVisible ? 'Hide project explorer' : 'Show project explorer',
        icon: 'sidebar',
        action: () => (explorerVisible = !explorerVisible),
      },
      {
        label: ide.bottomDockOpen ? 'Hide bottom tool dock' : 'Show bottom tool dock',
        icon: 'terminal',
        action: () => ide.toggleBottomDock(),
      },
      {
        label: ideAgent.panelOpen ? 'Hide AI Agent' : 'Show AI Agent',
        icon: 'assistant',
        hint: '⌘L',
        action: () => triggerAgentShortcut(),
      },
      { separator: true },
      {
        label: 'Add folder to project…',
        icon: 'folder',
        disabled: !ide.hasTarget || ide.target.kind !== 'local',
        action: () => (folderPickerMode = 'attach'),
      },
      { label: 'Toolchains & SDKs…', icon: 'box', action: () => (ide.toolchainsOpen = true) },
      { label: 'Environments & run configs…', icon: 'gear', action: () => ide.openEnvironmentsModal('env') },
      { label: 'Connect remote SSH…', icon: 'server', action: () => (ide.sshOpen = true) },
      { label: 'Share live session…', icon: 'share', action: () => (ide.shareOpen = true) },
      { label: 'IDE plugins…', icon: 'plug', action: () => (ide.pluginsOpen = true) },
      { separator: true },
      {
        label: 'Keymap: JetBrains',
        checked: ide.keymapPreset === 'jetbrains',
        action: () => ide.setKeymapPreset('jetbrains'),
      },
      {
        label: 'Keymap: Standard',
        checked: ide.keymapPreset === 'standard',
        action: () => ide.setKeymapPreset('standard'),
      },
    ];
    ctxMenu.show(e, items);
  }

  async function leaveOrEndCollab(): Promise<void> {
    if (ide.collabIsGuest) {
      ide.leaveCollabRoom();
      return;
    }
    const ok = await confirmer.ask('Disconnect all participants and revoke the share link?', {
      title: 'Stop sharing session?',
      confirmLabel: 'Stop sharing',
      danger: true,
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

<div class="ide-page" data-testid="ide-page">
  <PageHeader title="IDE">
    {#snippet badge()}
      <IdeWorkspaceSwitcher />
    {/snippet}

    {#snippet actions()}
      {#if ide.hasTarget}
        {#if !viewport.isPhone}
          <button
            type="button"
            class="icon-btn"
            data-overflow="1"
            data-icon="search"
            aria-label="Search everywhere"
            title="Search everywhere (⇧⇧)"
            onclick={() => {
              ide.searchScopePath = null;
              ide.searchReplaceMode = false;
              ide.searchInitialTab = 'all';
              ide.searchEverywhereOpen = true;
            }}
          >
            <Icon name="search" size={14} />
          </button>
        {/if}

        <IdeRunToolbar />

        {#if !viewport.isPhone}
          <button
            type="button"
            class="btn small ghost ide-agent-toggle"
            data-overflow="5"
            data-icon="assistant"
            data-label="Toggle AI Agent"
            aria-pressed={ideAgent.panelOpen}
            title="Toggle AI Agent (⌘L)"
            onclick={() => {
              ideAgent.togglePanel();
              if (ideAgent.panelOpen) focusAgentComposer();
            }}
            data-testid="ide-toggle-agent"
          >
            <Icon name="assistant" size={13} />
            Agent
            {#if ideAgent.contexts.length > 0}
              <Badge tone="accent" label={String(ideAgent.contexts.length)} />
            {/if}
          </button>

          <button
            type="button"
            class="btn small ghost"
            data-overflow="2"
            data-icon="share"
            onclick={() => (ide.shareOpen = true)}
            title="Share live IDE session with teammates"
            data-testid="ide-share-btn"
          >
            <Icon name="share" size={13} />
            Share
          </button>
        {/if}
      {/if}

      <button
        type="button"
        class="icon-btn"
        data-overflow="-10"
        data-icon="more"
        data-label="More actions…"
        aria-label="More actions"
        title="More actions"
        aria-haspopup="menu"
        onclick={openMoreMenu}
        data-testid="ide-more-btn"
      >
        <Icon name="more" size={14} />
      </button>
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
        body="Edit with multi-cursor power, manage SDKs and shell environments, set breakpoints in the DAP debugger, or code directly over SSH."
        actionLabel="Open local folder"
        actionIcon="folder"
        onaction={() => (folderPickerMode = 'open')}
      >
        <button type="button" class="btn ghost" onclick={() => (ide.sshOpen = true)}>
          <Icon name="server" size={13} />
          Connect over SSH…
        </button>
      </EmptyState>
    {:else if viewport.isPhone}
      <div class="ide-mobile-bar">
        <div class="segmented" role="group" aria-label="IDE mobile pane">
          <button
            type="button"
            class:active={mobileView === 'files'}
            aria-pressed={mobileView === 'files'}
            onkeydown={onTabKey}
            onclick={() => {
              mobileView = 'files';
              if (ideAgent.panelOpen) ideAgent.togglePanel(false);
            }}
          >
            Files
          </button>
          <button
            type="button"
            class:active={mobileView === 'editor'}
            aria-pressed={mobileView === 'editor'}
            onkeydown={onTabKey}
            onclick={() => {
              mobileView = 'editor';
              if (ideAgent.panelOpen) ideAgent.togglePanel(false);
            }}
          >
            Editor
          </button>
          <button
            type="button"
            class:active={mobileView === 'tools'}
            aria-pressed={mobileView === 'tools'}
            onkeydown={onTabKey}
            onclick={() => {
              mobileView = 'tools';
              ide.toggleBottomDock(true);
              if (ideAgent.panelOpen) ideAgent.togglePanel(false);
            }}
          >
            Tools
          </button>
          <button
            type="button"
            class:active={mobileView === 'agent'}
            aria-pressed={mobileView === 'agent'}
            onkeydown={onTabKey}
            onclick={() => {
              mobileView = 'agent';
              ideAgent.togglePanel(true);
            }}
            data-testid="ide-mobile-tab-agent"
          >
            Agent{#if ideAgent.contexts.length > 0} ({ideAgent.contexts.length}){/if}
          </button>
        </div>
      </div>

      <div class="ide-mobile-body">
        {#if mobileView === 'files'}
          <IdeProjectTree />
        {:else if mobileView === 'tools'}
          <IdeBottomDock />
        {:else if mobileView === 'agent'}
          <LazyMount lazy={IdeAgentPanelLazy} what="AI agent" variant="panel" />
        {:else}
          <IdeEditorArea />
        {/if}
      </div>
    {:else}
      <div class="ide-workbench">
        {#if explorerVisible}
          <div class="ide-rail-wrap" style:inline-size="{treeW}px">
            <IdeProjectTree />
          </div>
          <PaneDivider
            bind:width={treeW}
            storageKey={TREE_W_KEY}
            label="Resize project explorer"
            defaultWidth={270}
          />
        {/if}

        <div class="ide-center-col">
          <IdeEditorArea />
          <IdeBottomDock />
        </div>

        {#if ideAgent.panelOpen}
          <PaneDivider
            bind:width={agentW}
            storageKey={AGENT_W_KEY}
            min={280}
            max={560}
            defaultWidth={360}
            invert
            label="Resize AI agent dock"
          />
          <div class="ide-agent-wrap" style:inline-size="{agentW}px">
            <LazyMount lazy={IdeAgentPanelLazy} what="AI agent" variant="panel" />
          </div>
        {/if}
      </div>
    {/if}
  </PageBody>

  {#if folderPickerMode}
    <FolderPicker
      title={folderPickerMode === 'attach' ? 'Add folder to project' : 'Open project folder'}
      start={ide.target.kind === 'local' ? ide.target.root : ''}
      onpick={(p) => void handlePickedFolder(p)}
      onclose={() => (folderPickerMode = null)}
    />
  {/if}

  {#if ide.searchEverywhereOpen}
    <LazyMount lazy={IdeSearchEverywhereLazy} what="search everywhere" quiet onerror={() => (ide.searchEverywhereOpen = false)} />
  {/if}
  {#if ide.toolchainsOpen}
    <LazyMount lazy={IdeToolchainsModalLazy} what="toolchains manager" quiet onerror={() => (ide.toolchainsOpen = false)} />
  {/if}
  {#if ide.environmentsOpen}
    <LazyMount
      lazy={IdeEnvironmentsModalLazy}
      props={{ initialTab: ide.environmentsInitialTab }}
      what="environments"
      quiet
      onerror={() => (ide.environmentsOpen = false)}
    />
  {/if}
  {#if ide.sshOpen}
    <LazyMount lazy={IdeSshTargetModalLazy} what="SSH target picker" quiet onerror={() => (ide.sshOpen = false)} />
  {/if}
  {#if ide.shareOpen}
    <LazyMount lazy={IdeShareModalLazy} what="collaboration settings" quiet onerror={() => (ide.shareOpen = false)} />
  {/if}
  {#if ide.pluginsOpen}
    <LazyMount lazy={IdePluginsModalLazy} what="extensions manager" quiet onerror={() => (ide.pluginsOpen = false)} />
  {/if}
</div>

<style>
  .ide-page { display: flex; flex-direction: column; flex: 1; min-inline-size: 0; min-block-size: 0; block-size: 100%; }
  .ide-agent-toggle[aria-pressed='true'] {
    background: var(--accent-soft);
    border-color: var(--accent-line);
    color: var(--accent-text);
  }
  .ide-collab-banner {
    display: flex; align-items: center; gap: 8px; padding-block: 6px; padding-inline: 14px;
    background: var(--surface); border-block-end: 1px solid var(--border); font-size: var(--fs-xs); flex-wrap: wrap;
  }
  .collab-title { font-weight: 500; color: var(--text); }
  .collab-count { color: var(--text-dim); }
  .collab-peers { display: flex; align-items: center; gap: 4px; flex-wrap: wrap; }
  .collab-actions { display: flex; align-items: center; gap: 4px; margin-inline-start: auto; }
  .ide-workbench { display: flex; flex: 1; min-inline-size: 0; min-block-size: 0; }
  .ide-rail-wrap, .ide-agent-wrap {
    flex-shrink: 0; min-inline-size: 0; min-block-size: 0; block-size: 100%; display: flex; flex-direction: column;
  }
  .ide-center-col { flex: 1; min-inline-size: 0; min-block-size: 0; display: flex; flex-direction: column; }
  .ide-mobile-bar {
    display: flex; justify-content: center; padding-block: 6px; padding-inline: 10px;
    background: var(--surface); border-block-end: 1px solid var(--border);
  }
  .ide-mobile-bar .segmented { max-inline-size: 100%; }
  .ide-mobile-bar .segmented > button { padding-inline: 8px; }
  .ide-mobile-body { flex: 1; min-block-size: 0; min-inline-size: 0; display: flex; flex-direction: column; }
  @media (max-width: 1024px) {
    .ide-rail-wrap { max-inline-size: 32%; }
    .ide-agent-wrap { max-inline-size: 38%; }
  }
</style>
