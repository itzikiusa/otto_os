<script lang="ts">
  import { onDestroy, untrack } from 'svelte';
  import { announceShown } from '../../lib/announce.svelte';
  import { registry } from '../../lib/commands.svelte';
  import Badge from '../../lib/components/Badge.svelte';
  import EmptyState from '../../lib/components/EmptyState.svelte';
  import Icon from '../../lib/components/Icon.svelte';
  import LazyMount from '../../lib/components/LazyMount.svelte';
  import PageBody from '../../lib/components/PageBody.svelte';
  import PageHeader from '../../lib/components/PageHeader.svelte';
  import ListDetail from '../../lib/components/side/ListDetail.svelte';
  import { pageNav } from '../../lib/components/side/pageNav.svelte';
  import { confirmer } from '../../lib/confirm.svelte';
  import { keyContext } from '../../lib/keys';
  import { lazyComponent } from '../../lib/lazy-component.svelte';
  import { lazyModule } from '../../lib/lazyModule';
  import { plural } from '../../lib/plural';
  import { router } from '../../lib/router.svelte';
  import { ui } from '../../lib/stores/ui.svelte';
  import { pageChrome } from '../../lib/stores/pageChrome.svelte';
  import { ws } from '../../lib/stores/workspace.svelte';
  import { idePreferences } from '../../lib/idePreferences.svelte';
  import { ideWorkspacePort } from '../../lib/ideWorkspacePort.svelte';
  import type { IdeCompareMode } from './history/ideComparisonLoader';
  import IdeEditorArea from './IdeEditorArea.svelte';
  import { ideBreakpointsNav } from './debug/ideBreakpointsNav.svelte';
  import { ide, ideBasename } from './ideStore.svelte';
  import { findingPath, projectSecurity } from './security/projectSecurity.svelte';
  import type { SecurityFinding } from '../../lib/api/types';
  import type { IdeDiagnostic } from './store/ideTypes';
  import IdeWorkspaceSwitcher from './shell/IdeWorkspaceSwitcher.svelte';
  import { loadScratchesRoot, newScratchFile, openScratches } from './shell/ideScratches.svelte';
  import IdeHeaderActions from './shell/IdeHeaderActions.svelte';
  import { IDE_TYPING_SELECTOR, idePhoneChrome } from './editor/idePhoneChrome.svelte';
  import { EXPLORER_VIEW_LABEL, ideLayout, paneAfterFileOpen, STACK_MAX_H, type IdeStackPane } from './shell/ideLayout.svelte';
  import { ideTabRestore } from './shell/ideProjectHelpers';
  import {
    handleIdeChord,
    handleIdeWindowKeyDown,
    promptIdeGoToLine,
    readQuickSchemePrefs,
  } from './shell/ideShellCommands';
  import { ideAgent } from './store/ideAgentController.svelte';

  const loadShellMenus = () => import('./shell/ideShellMenus');

  const IdeSecurityPageLazy = lazyComponent(() => import('./security/IdeSecurityPage.svelte'));
  const IdeProjectTreeLazy = lazyComponent(() => import('./IdeProjectTree.svelte'));
  const FolderPickerLazy = lazyComponent(() => import('../../lib/components/FolderPicker.svelte'));
  const agentPort = lazyModule(() => import('./agentDebug/ideAgentPort.svelte'));
  const IdeBottomDockLazy = lazyComponent(() => import('./IdeBottomDock.svelte'));
  const IdeStatusStripLazy = lazyComponent(() => import('./editor/IdeStatusStrip.svelte'));
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
  const IdePhoneTabRowLazy = lazyComponent(() => import('./editor/IdePhoneTabRow.svelte'));

  let quickPrefs = $state(readQuickSchemePrefs());
  let securityOpen = $state(false);
  $effect(() => {
    const id = ide.activeProjectId;
    const allowed = !!id && !ide.collabIsGuest && ide.target.kind === 'local';
    const controller = new AbortController();
    projectSecurity.problems = [];
    if (allowed && id) {
      void import('./security/projectSecurityData')
        .then(async ({ latestSecurityDiagnostics }) => {
          if (controller.signal.aborted) return;
          const items = await latestSecurityDiagnostics(id, controller.signal);
          if (!controller.signal.aborted) projectSecurity.problems = items;
        })
        .catch(() => {});
    }
    return () => controller.abort();
  });

  function openSecurityFinding(finding: SecurityFinding) {
    securityOpen = false;
    showPane('editor');
    void ide.openFile(findingPath(finding), { line: finding.line ?? 1, preview: false });
  }

  function draftSecurityFix(finding: SecurityFinding) {
    securityOpen = false;
    ideAgent.composerText = [
      ideAgent.composerText.trimEnd(),
      `Review and propose a fix for ${finding.rule} from ${finding.engine}.`,
      finding.check === 'dast' ? `URL ${finding.path}` : `File ${findingPath(finding)}${finding.line ? ` at line ${finding.line}` : ''}`,
      finding.description,
      finding.fix,
    ].filter(Boolean).join('\n');
    ideAgent.togglePanel(true);
    focusAgentComposer();
  }
  // Stacked layout (a phone, a phone on its side): the explorer is the page and
  // a file is pushed over it (ListDetail); shell/ideLayout.svelte.ts.
  const stack = $derived(ideLayout.stack);
  // Project tabs take the header's tabs slot on wide layouts only; stacked
  // (phones) keeps the dropdown.
  const projectTabsOn = $derived(!stack && !ide.collabIsGuest && idePreferences.current.projectTabs);
  const pane = $derived(ideLayout.pane);
  // One owner for the page's chrome: the workbench's status strip replaces the shell's (a tablet
  // shows Otto's Rail by default, so an iPad portrait has room for the workbench, I4-24).
  $effect(() => pageChrome.set({ hideStatusBar: !stack }));
  // A live-session guest works only in the host's open files (no tree, run, agent or
  // terminal routes exist for guests): those controls stay out of its page.
  // "Breakpoints (n)…" in the debugger opens the tree's Breakpoints tab: a hidden
  // explorer (the list pane) comes back for it.
  let bpNavSeen = ideBreakpointsNav.seq;
  $effect(() => {
    const seq = ideBreakpointsNav.seq;
    if (seq === bpNavSeen) return;
    bpNavSeen = seq;
    untrack(() => pageNav.show?.onclick());
  });
  // The agent is the page's Detail pane (a full-width sheet on a phone).
  const agentShown = $derived(!ide.collabIsGuest && ideAgent.panelOpen);
  function toggleAgentDock(): void {
    ideAgent.togglePanel();
    if (ideAgent.panelOpen) focusAgentComposer();
  }
  // ⌥⌘B: the Detail pane's toggle (it opens it too, so the page claims it).
  $effect(() => (ide.collabIsGuest ? undefined : pageNav.claimDetail(toggleAgentDock)));
  // Typing (code or agent composer) with the on-screen keyboard up: the page header
  // steps aside so the code keeps its lines (editor/idePhoneChrome.svelte.ts).
  const phone = $derived(idePhoneChrome.current);
  let folderPickerMode = $state<'open' | 'attach' | null>(null);
  let compareTarget = $state<{
    mode: IdeCompareMode;
    root: string;
    targetPath: string;
    secondPath: string | null;
    isDir: boolean;
    initialFilter: 'all' | 'deleted';
    initialRef: string | null;
    initialClipboardText?: string; sourceLabel?: string | null;
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
    clipboardText?: string; leftLabel?: string;
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
      initialClipboardText: opts.clipboardText, sourceLabel: opts.leftLabel ?? null,
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
      const d = (ev as CustomEvent<{ mode?: IdeCompareMode; path?: string; secondPath?: string; isDir?: boolean; filter?: 'all' | 'deleted'; ref?: string; clipboardText?: string; leftLabel?: string } | undefined>).detail;
      openCompareModal({ mode: d?.mode ?? 'local-history', path: d?.path, secondPath: d?.secondPath, isDir: d?.isDir, filter: d?.filter, ref: d?.ref, clipboardText: d?.clipboardText, leftLabel: d?.leftLabel });
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

  $effect(() => ideWorkspacePort.setAgentDockOpen(ideAgent.panelOpen));

  function showPane(next: IdeStackPane): void {
    ideLayout.pane = next;
  }

  // A file opened from the Files pane (a tap in the tree) shows the editor; the tabs a
  // project restores on load leave the pane the person already picked.
  let lastActivePath = ide.activePath;
  $effect(() => {
    const p = ide.activePath;
    if (p === lastActivePath) return;
    lastActivePath = p;
    untrack(() => {
      if (p && ideLayout.stack && !ideTabRestore.active) ideLayout.pane = paneAfterFileOpen(ideLayout.pane);
    });
  });

  $effect(() => {
    void ws.currentId;
    void ws.workspaces.length;
    void ws.listSettled;
    untrack(() => ide.attachDefaultWorkspace());
  });

  // Know the Scratches folder up front: the switcher names it "Scratches"
  // (a reload may restore it as the last project).
  loadScratchesRoot().catch(() => {});
  // Agent debugging (`otto.ui_ide_debug_*`) drives this IDE through the port. Its code
  // loads after the page (not in the IDE chunk); the handlers wait for the bind.
  $effect(() => {
    let unbind: (() => void) | null = null;
    let mounted = true;
    agentPort.use((m) => {
      if (mounted) unbind = m.bindIdeAgentPort();
    });
    return () => {
      mounted = false;
      unbind?.();
    };
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
    const scratchesParam = params.get('scratches');
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
          if (scratchesParam === 'new') await newScratchFile();
          else if (scratchesParam) await openScratches();
          else if (projectParam) await ide.activateProject(projectParam);
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
      handleIdeChord(e, ui.overlayOpen, () => pageNav.toggle?.(), () => triggerAgentShortcut());
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
    idePhoneChrome.typing = false;
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
  class:typing={phone.typingMode}
  onfocusin={(e) => (idePhoneChrome.typing = e.target instanceof Element && !!e.target.closest(IDE_TYPING_SELECTOR))}
  onfocusout={() => (idePhoneChrome.typing = false)}
  data-pane={stack ? pane : undefined}
  data-testid="ide-page"
  data-ide-density={idePreferences.current.uiDensity}
  bind:clientWidth={ideLayout.width}
  bind:clientHeight={ideLayout.height}
>
  {#if securityOpen && ide.activeProjectId && ide.activeProject && !ide.collabIsGuest && ide.target.kind === 'local'}
    {#key ide.activeProjectId}
      {@const projectId = ide.activeProjectId}
      <LazyMount lazy={IdeSecurityPageLazy} what="project security" variant="panel" props={{
        projectId, projectName: ide.activeProject.name,
        onBack: () => { securityOpen = false; },
        onOpen: openSecurityFinding, onDraft: draftSecurityFix,
        onDiagnostics: (items: IdeDiagnostic[]) => { if (ide.activeProjectId === projectId) projectSecurity.problems = items; },
      }} />
    {/key}
  {:else}
  <!-- One toolbar row (G13). A phone: the explorer is the page (its views are
       the header) and a file is pushed with "‹ Files"; a phone on its side
       keeps the open files inline in the row. Wide, the optional project tabs
       (Settings › Appearance) take the tabs slot. -->
  {#snippet phoneFiles()}<LazyMount lazy={IdePhoneTabRowLazy} what="file switcher" quiet props={{ inHeader: true }} />{/snippet}
  {#snippet projectTabs()}<IdeWorkspaceSwitcher mode="tabs" onpickfolder={(m) => (folderPickerMode = m)} />{/snippet}
  <!-- A phone: the project switcher is the Files list's footer (its name heads the tree). -->
  {#snippet projectFooter()}<IdeWorkspaceSwitcher onpickfolder={(m) => (folderPickerMode = m)} />{/snippet}
  <PageHeader
    title={stack ? (ide.activePath && pane === 'editor' ? ideBasename(ide.activePath) : 'IDE') : 'IDE (Beta)'}
    tabs={stack ? (ide.hasTarget && phone.fileRowInHeader && pane === 'editor' ? phoneFiles : undefined) : projectTabsOn ? projectTabs : undefined}
  >
    {#snippet badge()}
      {#if !ide.collabIsGuest && !projectTabsOn && !stack}<IdeWorkspaceSwitcher onpickfolder={(m) => (folderPickerMode = m)} />{/if}
    {/snippet}

    {#snippet actions()}
      {#if !stack && ide.activeProjectId && !ide.collabIsGuest && ide.target.kind === 'local'}
        <button type="button" class="btn small ghost" data-icon="shield" onclick={() => { securityOpen = true; }}>
          <Icon name="shield" size={14} /> Security
        </button>
      {/if}
      {#if !ide.collabIsGuest}<IdeHeaderActions
        {stack}
        bind:quickPrefs
        onTriggerAgent={() => triggerAgentShortcut()}
        onAttachFolder={() => (folderPickerMode = 'attach')}
        onShowPane={showPane}
        onOpenScratches={() => void openScratches()}
        onOpenSecurity={ide.activeProjectId && ide.target.kind === 'local' ? () => (securityOpen = true) : undefined}
        onToggleAgentDock={toggleAgentDock}
      />{/if}
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
    <div class="ide-collab-banner" use:announceShown={{ text: ide.collabError, assertive: true }} data-testid="ide-collab-ended-banner">
      <Badge tone="warn" label="Session ended" />
      <span class="collab-title">{ide.collabError}</span>
      <div class="collab-actions">
        <button
          type="button"
          class="btn small"
          onclick={() => { ide.leaveCollabRoom(); ide.attachDefaultWorkspace(); }}
          data-testid="ide-collab-leave-ended"
        >
          Leave session
        </button>
      </div>
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
        <button type="button" class="btn ghost" onclick={() => void openScratches()} data-testid="ide-open-scratches">
          <Icon name="workbench" size={13} />
          Open Scratches
        </button>
      </EmptyState>
    {:else}
      <!-- The explorer is the list pane (its views are the header: Files |
           Structure | Breakpoints; ⋯ › Hide <view> ⌘B); on a phone it is the
           page and a file is pushed with "‹ Files". The build tools and the
           agent are Detail panes. Both sides stay mounted, so Back or a
           rotation never remounts the editor, terminal or agent. -->
      <ListDetail
        module="ide"
        name={EXPLORER_VIEW_LABEL[ideLayout.explorerView]}
        open={pane !== 'files'}
        onback={() => showPane('files')}
        empty={ide.collabIsGuest}
        legacy={{ w: 'otto.ide.treeW' }}
      >
        {#snippet list()}
          <LazyMount lazy={IdeProjectTreeLazy} what="project explorer" variant="panel" props={{ onOpenCompare: openCompareModal, footer: stack ? projectFooter : undefined }} />
        {/snippet}
        <div class="ide-center-col" class:tools-only={stack && pane === 'tools'}>
          <IdeEditorArea />
          <LazyMount lazy={IdeBottomDockLazy} what="bottom tool dock" quiet props={{ stack, stackShowing: stack && pane === 'tools' }} />
          {#if !stack}<LazyMount lazy={IdeStatusStripLazy} what="status bar" quiet />{/if}
        </div>
        {#snippet detail()}
          {#if ide.buildToolOpen && !ide.collabIsGuest}
            <LazyMount lazy={IdeBuildToolWindowLazy} what="build tool window" quiet />
          {/if}
          {#if agentShown}
            <LazyMount lazy={IdeAgentPanelLazy} what="AI agent" quiet />
          {/if}
        {/snippet}
      </ListDetail>
    {/if}
  </PageBody>
  {/if}

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
  .ide-center-col { flex: 1; min-inline-size: 0; min-block-size: 0; display: flex; flex-direction: column; }
  /* A phone's Tools: the dock fills the content (the editor stays mounted). */
  .ide-center-col.tools-only :global(.ide-editor-area) { display: none; }
  /* Stacked chrome: no fold column, no status strip; on a short page (phone on
     its side) no sub-bar. */
  /* CodeMirror's base theme declares `.cm-gutter { display: flex !important }`. */
  .ide-page.stack :global(.cm-gutter.cm-foldGutter) { display: none !important; }
  /* A short stacked page keeps its rows for code, unless the person asked for the toolbar (tab ▾). */
  .ide-page.short-stack :global(.ide-subbar:not(.asked)) { display: none; }
  .ide-page.typing :global(.page-header-bar), .ide-page.typing :global(.ide-subbar) { display: none; }
</style>
