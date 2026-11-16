<script lang="ts">
  // Full DAP Debugger panel: ONE calm toolbar row (execution controls, status
  // + paused location, view switch, and a ⋯ for everything else), Tabbed vs
  // Split layout, one inline failure card with a next step, and a Debug
  // Console that reads like a terminal (stitched lines, evaluations in time).
  import { onMount, untrack } from 'svelte';
  import Badge from '../../lib/components/Badge.svelte';
  import EmptyState from '../../lib/components/EmptyState.svelte';
  import Icon from '../../lib/components/Icon.svelte';
  import LoadState from '../../lib/components/LoadState.svelte';
  import { api } from '../../lib/api/client';
  import { ctxMenu, type MenuItem } from '../../lib/contextmenu.svelte';
  import { idePreferences } from '../../lib/idePreferences.svelte';
  import { pollWhileVisible } from '../../lib/poll';
  import type { BadgeTone } from '../../lib/status';
  import {
    ide,
    ideBasename,
    type IdeDebugSessionSnapshot,
  } from './ideStore.svelte';
  import { ideAgent } from './store/ideAgentController.svelte';
  import { ideDebugLaunch } from './store/ideDebugLaunch.svelte';
  import {
    areIdeBreakpointsMuted,
    toggleIdeBreakpointsMuted,
  } from './editor/ideEditorStoreBridge.svelte';
  import { ideLayout } from './shell/ideLayout.svelte';
  import { ideChordLabel, ideKeymapPresetOf, ideShortcutChords, type IdeShortcutId } from './shell/ideShortcuts';
  import IdeConsoleView, { type ConsoleLineInput } from './debug/IdeConsoleView.svelte';
  import IdeDebugErrorCard from './debug/IdeDebugErrorCard.svelte';
  import IdeDebuggerInspector from './debug/IdeDebuggerInspector.svelte';
  import { openBreakpointsList } from './debug/ideBreakpointsNav.svelte';
  import { lastOutputSeq, OutputSeqTracker, stitchOutput } from './debug/ideConsoleModel';
  import { chordAria, debugErrorCard, pickReattachSession, withKeyHint } from './debug/ideDebugPanelModel';
  import { ideDockPrefs } from './debug/ideDockPrefs.svelte';
  import { ideCollabNotices } from './store/ideCollabNotices.svelte';

  let activeSubTab = $state<'debugger' | 'console'>('debugger');
  let replInput = $state('');
  let replHistory = $state<string[]>([]);
  let historyIdx = $state(-1);
  let evaluating = $state(false);
  /** Console position of evaluations on daemons that don't send `seq`. */
  let replAt = $state<Record<string, number>>({});
  /** A session failure the person dismissed (by session id). */
  let dismissedErrorFor = $state('');

  const dbg = $derived(ide.activeDebug);
  const compact = $derived(ideLayout.stack);

  // Output chunks → lines. Sequence numbers come from the daemon (`seq`), else
  // from a client counter that follows the window as the daemon trims it.
  const seqTracker = new OutputSeqTracker();
  const outSeqs = $derived(seqTracker.next(dbg?.id ?? '', dbg?.output ?? []));
  const consoleLines = $derived<readonly ConsoleLineInput[]>(
    stitchOutput(dbg?.output ?? [], outSeqs).map((l) => ({ text: l.text, isError: l.isError, seq: l.seq, endSeq: l.endSeq, key: l.key })),
  );
  const lastSeq = $derived(outSeqs.length > 0 ? outSeqs[outSeqs.length - 1] : 0);
  const replEntries = $derived(
    ide.replEntries.map((e) => (typeof e.afterSeq === 'number' || replAt[e.id] === undefined ? e : { ...e, afterSeq: replAt[e.id] })),
  );
  const consoleCount = $derived(consoleLines.length + ide.replEntries.length);
  const latestRepl = $derived(
    ide.replEntries.length > 0 ? ide.replEntries[ide.replEntries.length - 1] : null,
  );
  const isPaused = $derived(dbg?.state === 'paused');
  const isRunning = $derived(dbg?.state === 'running' || dbg?.state === 'starting');
  const isLive = $derived(Boolean(dbg && dbg.state !== 'terminated' && dbg.state !== 'error'));

  const errorCard = $derived.by(() => {
    const card = debugErrorCard(ideDebugLaunch.startError, dbg && dbg.id !== dismissedErrorFor ? dbg : null);
    return card;
  });

  const statusBadge = $derived.by((): { label: string; tone: BadgeTone; live: boolean } => {
    if (!dbg) return { label: 'Inactive', tone: 'neutral', live: false };
    if (dbg.state === 'paused') {
      const why = dbg.stopped_reason ? ` · ${dbg.stopped_reason}` : '';
      return { label: `Paused${why}`, tone: 'warn', live: false };
    }
    if (dbg.state === 'running') return { label: 'Running', tone: 'info', live: true };
    if (dbg.state === 'starting') return { label: 'Starting', tone: 'info', live: true };
    if (dbg.state === 'error') return { label: 'Error', tone: 'bad', live: false };
    return { label: 'Terminated', tone: 'neutral', live: false };
  });

  // Tooltips and aria-keyshortcuts follow the keymap preset (one shared table).
  const preset = $derived(ideKeymapPresetOf(idePreferences.current.keymapPreset));
  function tip(id: IdeShortcutId, title: string): string {
    return compact ? title : withKeyHint(title, ideShortcutChords(id, preset).map(ideChordLabel));
  }
  function keys(id: IdeShortcutId): string | undefined {
    const chords = ideShortcutChords(id, preset);
    return chords.length > 0 ? chords.map(chordAria).join(' ') : undefined;
  }

  onMount(() => {
    if (ide.activeDebug) return;
    const ctl = new AbortController();
    void api.bg
      .get<{ sessions: IdeDebugSessionSnapshot[] }>('/ide/debug/sessions', ctl.signal)
      .then((list) => {
        if (ide.activeDebug) return;
        // Only a live session launched from THIS project (a finished one, or
        // another project's, would open its files here and seed Rerun wrongly).
        const live = pickReattachSession(list.sessions ?? [], ide.target.root);
        if (live) ide.applyDebugSnapshot(live);
      })
      .catch(() => {});
    return () => ctl.abort();
  });

  $effect(() => {
    if (!dbg || dbg.state === 'terminated' || dbg.state === 'error' || ide.debugStreamActive) return;
    const sessionId = dbg.id;
    const poller = pollWhileVisible(
      async (signal) => {
        const cur = untrack(() => ide.activeDebug);
        if (!cur || cur.id !== sessionId || cur.state === 'terminated' || cur.state === 'error') return;
        try {
          const res = await api.bg.get<{ session: IdeDebugSessionSnapshot }>(
            `/ide/debug/sessions/${encodeURIComponent(sessionId)}`,
            signal,
          );
          const next = res.session;
          if (!next) return;
          const nOut = next.output ?? [];
          const cOut = cur.output ?? [];
          const changed =
            next.revision !== cur.revision ||
            next.state !== cur.state ||
            next.active_frame_id !== cur.active_frame_id ||
            next.paused_file !== cur.paused_file ||
            next.paused_line !== cur.paused_line ||
            (next.stack_frames?.length ?? 0) !== (cur.stack_frames?.length ?? 0) ||
            (next.scopes?.length ?? 0) !== (cur.scopes?.length ?? 0) ||
            nOut.length !== cOut.length ||
            // At the output cap the length stays put: compare the tail too.
            lastOutputSeq(nOut) !== lastOutputSeq(cOut) ||
            nOut[nOut.length - 1]?.text !== cOut[cOut.length - 1]?.text;
          if (changed) ide.applyDebugSnapshot(next);
        } catch { /* transient or ended */ }
      },
      { ms: 900, floorMs: 500, immediate: false },
    );
    return () => poller.stop();
  });

  async function onEvalSubmit(e: SubmitEvent): Promise<void> {
    e.preventDefault();
    const expr = replInput.trim();
    if (!expr || !dbg || evaluating) return;
    evaluating = true;
    replHistory = [expr, ...replHistory.filter((h) => h !== expr)].slice(0, 50);
    historyIdx = -1;
    replInput = '';
    const at = lastSeq;
    const before = new Set(ide.replEntries.map((r) => r.id));
    try { await ide.evaluateDebug(expr, 'repl', dbg.active_frame_id ?? undefined); }
    finally {
      evaluating = false;
      const added = ide.replEntries.filter((r) => !before.has(r.id) && typeof r.afterSeq !== 'number');
      if (added.length > 0) replAt = { ...replAt, ...Object.fromEntries(added.map((r) => [r.id, at])) };
    }
  }

  function onReplKeydown(e: KeyboardEvent): void {
    if (e.key === 'ArrowUp' && replHistory.length > 0) {
      e.preventDefault();
      historyIdx = Math.min(replHistory.length - 1, historyIdx + 1);
      replInput = replHistory[historyIdx] ?? '';
    } else if (e.key === 'ArrowDown' && historyIdx >= 0) {
      e.preventDefault();
      historyIdx -= 1;
      replInput = historyIdx >= 0 ? (replHistory[historyIdx] ?? '') : '';
    }
  }

  function setExceptions(next: boolean): void {
    ide.breakOnExceptions = next;
    void ide.syncBreakpointsToSession();
  }

  function showConsole(): void {
    if (ideDockPrefs.debuggerLayout === 'tabbed') activeSubTab = 'console';
    else if (!ideDockPrefs.dbgShowConsole) ideDockPrefs.toggleConsolePane();
  }

  function dismissError(): void {
    ideDebugLaunch.startError = null;
    if (dbg?.error) dismissedErrorFor = dbg.id;
  }

  function retryLaunch(): void {
    dismissError();
    void ide.startDebug();
  }

  function openMore(e: MouseEvent): void {
    const tabbed = ideDockPrefs.debuggerLayout === 'tabbed';
    const items: MenuItem[] = [
      { label: 'Tabbed layout', checked: tabbed, action: () => { if (!tabbed) ideDockPrefs.toggleDebuggerLayout(); } },
      { label: 'Split columns', checked: !tabbed, action: () => { if (tabbed) ideDockPrefs.toggleDebuggerLayout(); } },
      { separator: true },
      ...(tabbed
        ? [
            { label: 'Debugger view', checked: activeSubTab === 'debugger', action: () => (activeSubTab = 'debugger') },
            { label: 'Console view', checked: activeSubTab === 'console', action: () => (activeSubTab = 'console') },
            { label: 'Frames column', checked: ideDockPrefs.dbgShowFrames, action: () => ideDockPrefs.toggleFrames() },
          ]
        : [
            { label: 'Frames column', checked: ideDockPrefs.dbgShowFrames, action: () => ideDockPrefs.toggleFrames() },
            { label: 'Variables column', checked: ideDockPrefs.dbgShowVariables, action: () => ideDockPrefs.toggleVariables() },
            { label: 'Console column', checked: ideDockPrefs.dbgShowConsole, action: () => ideDockPrefs.toggleConsolePane() },
          ]),
      { separator: true },
      { label: 'Mute breakpoints', checked: areIdeBreakpointsMuted(), action: () => toggleIdeBreakpointsMuted() },
      { label: 'Break on exceptions', checked: ide.breakOnExceptions, action: () => setExceptions(!ide.breakOnExceptions) },
      { label: `Breakpoints (${ide.allBreakpoints.length})…`, icon: 'dot', action: () => void openBreakpointsList() },
      { separator: true },
      {
        label: 'Ask Agent about this stop',
        icon: 'assistant',
        disabled: !isPaused || !dbg,
        title: isPaused ? undefined : 'Pause at a breakpoint to ask the agent',
        action: () => dbg && ideAgent.attachDebugFrame(dbg, ide.expandedVariables, { openPanel: true }),
      },
    ];
    ctxMenu.show(e, items);
  }
</script>

{#snippet evalBar(showLatestPreview: boolean)}
  {#if showLatestPreview && latestRepl}
    <div class="dbg-quick-repl">
      <span class="dbg-repl-expr mono">&gt; {latestRepl.expression}:</span>
      <span class="dbg-repl-val mono ellipsis" class:is-err={latestRepl.isError} title={latestRepl.result}>{latestRepl.result}</span>
      <button type="button" class="dbg-pane-btn" onclick={showConsole} title="Open full Console output">
        Console ({consoleCount})
      </button>
    </div>
  {/if}
  <form class="dbg-repl-form" onsubmit={(e) => void onEvalSubmit(e)}>
    <input
      class="input dbg-inline-input mono"
      type="text"
      dir="ltr"
      aria-label="Evaluate expression"
      placeholder="Evaluate expression in current frame…"
      bind:value={replInput}
      onkeydown={onReplKeydown}
      disabled={!isLive}
    />
    <button type="submit" class="btn small" disabled={!isLive || !replInput.trim() || evaluating}>
      {evaluating ? 'Evaluating…' : 'Evaluate'}
    </button>
  </form>
{/snippet}

<div class="dbg-root" class:is-compact={compact} data-testid="ide-debugger-panel">
  <header class="dbg-bar" data-testid="ide-debug-bar">
    <div class="dbg-controls" role="toolbar" aria-label="Debugger execution controls">
      {#if isRunning}
        <button type="button" class="icon-btn" aria-label="Pause program" title="Pause program" onclick={() => void ide.pauseDebug()} data-testid="ide-debug-pause"><Icon name="pause" size={13} /></button>
      {:else}
        <button type="button" class="icon-btn" aria-label={tip('resume', 'Resume program')} title={tip('resume', 'Resume program')} aria-keyshortcuts={keys('resume')} disabled={!isPaused} onclick={() => void ide.resumeDebug()} data-testid="ide-debug-resume"><Icon name="play" size={13} /></button>
      {/if}
      <span class="dbg-sep" aria-hidden="true"></span>
      <button type="button" class="icon-btn" aria-label={tip('stepOver', 'Step over')} title={tip('stepOver', 'Step over')} aria-keyshortcuts={keys('stepOver')} disabled={!isPaused} onclick={() => void ide.stepOver()} data-testid="ide-debug-step-over"><Icon name="stepOver" size={13} /></button>
      <button type="button" class="icon-btn" aria-label={tip('stepInto', 'Step into')} title={tip('stepInto', 'Step into')} aria-keyshortcuts={keys('stepInto')} disabled={!isPaused} onclick={() => void ide.stepInto()} data-testid="ide-debug-step-into"><Icon name="stepInto" size={13} /></button>
      <button type="button" class="icon-btn" aria-label={tip('stepOut', 'Step out')} title={tip('stepOut', 'Step out')} aria-keyshortcuts={keys('stepOut')} disabled={!isPaused} onclick={() => void ide.stepOut()} data-testid="ide-debug-step-out"><Icon name="stepOut" size={13} /></button>
      <button type="button" class="icon-btn" aria-label={tip('runToCursor', 'Run to cursor')} title={tip('runToCursor', 'Run to cursor')} aria-keyshortcuts={keys('runToCursor')} disabled={!isPaused || !ide.activePath} onclick={() => void ide.runToCursor()}><Icon name="target" size={13} /></button>
      <span class="dbg-sep" aria-hidden="true"></span>
      <!-- A guest can't restart the host's session (it relaunches the host's program, as the host). -->
      <button type="button" class="icon-btn" aria-label={tip('restartDebug', 'Restart debug session')} title={ide.collabIsGuest ? 'Only the host can restart the debug session' : tip('restartDebug', 'Restart debug session')} aria-keyshortcuts={keys('restartDebug')} disabled={ide.collabIsGuest || (!isLive && !ide.canRestartDebug) || ide.debugLoading} onclick={() => void ide.restartDebug()} data-testid="ide-debug-restart"><Icon name="refresh" size={13} /></button>
      <button type="button" class="icon-btn dbg-stop" class:is-live={isLive} aria-label={tip('stop', 'Stop debug session')} title={tip('stop', 'Stop debug session')} aria-keyshortcuts={keys('stop')} disabled={!isLive} onclick={() => void ide.stopDebug()} data-testid="ide-debug-stop"><Icon name="stop" size={13} /></button>
    </div>

    <span class="dbg-host-only" role="status" data-testid="ide-debug-host-only">{ide.collabIsGuest ? (ideCollabNotices.debug ?? '') : ''}</span>

    {#if dbg}
      <div class="dbg-meta">
        <Badge tone={statusBadge.tone} label={statusBadge.label} dot live={statusBadge.live} status={dbg.state} title={`${statusBadge.label} — ${dbg.language} · ${dbg.adapter}`} testid="ide-debug-status" />
        {#if dbg.paused_file && dbg.paused_line}
          <button
            type="button"
            class="btn small ghost mono dbg-compact-btn dbg-loc"
            dir="ltr"
            title={`Jump to paused line ${dbg.paused_file}:${dbg.paused_line}`}
            onclick={() => dbg.paused_file && dbg.paused_line && void ide.openFile(dbg.paused_file, { line: dbg.paused_line, preview: false })}
            data-testid="ide-debug-location"
          >{ideBasename(dbg.paused_file)}:{dbg.paused_line}</button>
        {/if}
      </div>
    {/if}

    <span class="grow"></span>

    {#if dbg && ideDockPrefs.debuggerLayout === 'tabbed'}
      <div class="segmented dbg-subtabs" role="group" aria-label="Debugger sub-views">
        <button type="button" class:active={activeSubTab === 'debugger'} aria-pressed={activeSubTab === 'debugger'} onclick={() => (activeSubTab = 'debugger')} data-testid="ide-debug-subtab-debugger">Debugger</button>
        <button type="button" class:active={activeSubTab === 'console'} aria-pressed={activeSubTab === 'console'} onclick={() => (activeSubTab = 'console')} data-testid="ide-debug-subtab-console">Console{consoleCount > 0 ? ` (${consoleCount})` : ''}</button>
      </div>
    {/if}
    <button type="button" class="icon-btn dbg-more" aria-label="More debugger options" title="More debugger options" aria-haspopup="menu" onclick={openMore} data-testid="ide-debug-more">
      <Icon name="more" size={13} />
    </button>
  </header>

  {#if errorCard}
    <IdeDebugErrorCard card={errorCard} onshowoutput={showConsole} ondismiss={dismissError} onretry={retryLaunch} />
  {/if}

  {#if !dbg}
    {#if !errorCard}
      <div class="dbg-empty-wrap">
        <LoadState what="debug session" loading={ide.debugLoading} empty={true} variant="panel" rows={3}>
          {#snippet emptyView()}
            <EmptyState
              icon="bug"
              title="No active debug session"
              body="Set a breakpoint in the gutter and click Debug, or click the bug icon next to any test."
              actionLabel={ide.hasTarget ? 'Start debugging' : undefined}
              actionIcon="bug"
              actionKind="secondary"
              onaction={ide.hasTarget ? () => void ide.startDebug() : undefined}
              headingLevel={2}
            />
          {/snippet}
        </LoadState>
      </div>
    {/if}
  {:else if ideDockPrefs.debuggerLayout === 'tabbed'}
    {#if activeSubTab === 'console'}
      <div class="dbg-full-console" data-testid="ide-debug-console-full">
        <IdeConsoleView
          sessionKey={dbg.id}
          lines={consoleLines}
          {replEntries}
          emptyText="Program output and evaluated expressions appear here."
          clearLabel="Clear debug console and evaluate history"
          onclear={() => ide.clearRepl()}
        >
          {#snippet footer()}{@render evalBar(false)}{/snippet}
        </IdeConsoleView>
      </div>
    {:else}
      <div class="dbg-flex">
        <IdeDebuggerInspector
          {dbg}
          showFrames={ideDockPrefs.dbgShowFrames}
          showVariables={true}
          hasTrailingConsole={false}
        >
          {#snippet quickEvalSlot()}{@render evalBar(true)}{/snippet}
        </IdeDebuggerInspector>
      </div>
    {/if}
  {:else}
    <div class="dbg-flex" data-testid="ide-debug-split-grid">
      <IdeDebuggerInspector
        {dbg}
        showFrames={ideDockPrefs.dbgShowFrames}
        showVariables={ideDockPrefs.dbgShowVariables}
        hasTrailingConsole={ideDockPrefs.dbgShowConsole}
      >
        {#snippet quickEvalSlot()}
          {#if !ideDockPrefs.dbgShowConsole}{@render evalBar(true)}{/if}
        {/snippet}
      </IdeDebuggerInspector>

      {#if ideDockPrefs.dbgShowConsole}
        <section class="dbg-console-col" aria-label="Debug console and evaluate" data-testid="ide-debug-console-pane">
          <IdeConsoleView
            sessionKey={dbg.id}
            lines={consoleLines}
            {replEntries}
            title="Console"
            emptyText="Program output and evaluated expressions appear here."
            clearLabel="Clear debug console and evaluate history"
            onclear={() => ide.clearRepl()}
          >
            {#snippet footer()}{@render evalBar(false)}{/snippet}
          </IdeConsoleView>
        </section>
      {/if}
    </div>
  {/if}
</div>

<style>
  .dbg-host-only { font-size: var(--fs-xs); color: var(--text-dim); white-space: nowrap; overflow: hidden; text-overflow: ellipsis; min-inline-size: 0; flex-shrink: 1; }
  .dbg-host-only:empty { display: none; }
  .dbg-root { container-type: inline-size; display: flex; flex-direction: column; block-size: 100%; min-block-size: 0; min-inline-size: 0; background: var(--surface); }
  /* One row that never scrolls sideways: the controls never shrink, the
     status/location give way first (ellipsis, then hidden). */
  .dbg-bar {
    display: flex; align-items: center; gap: 6px;
    block-size: 32px; min-block-size: 32px; max-block-size: 32px;
    padding-inline: 8px; border-block-end: 1px solid var(--border);
    background: var(--surface); flex-shrink: 0; overflow: hidden;
  }
  .dbg-controls { display: flex; align-items: center; gap: 2px; flex-shrink: 0; }
  .dbg-meta { display: flex; align-items: center; gap: 4px; min-inline-size: 0; flex: 0 1 auto; overflow: hidden; }
  .dbg-meta :global(.badge) { min-inline-size: 0; }
  .dbg-loc { min-inline-size: 0; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; flex: 0 1 auto; }
  .dbg-compact-btn { block-size: 22px; padding-block: 0; }
  .dbg-subtabs { block-size: 22px; flex-shrink: 0; }
  .dbg-subtabs button { padding-block: 0; padding-inline: 8px; font-size: var(--fs-xs); white-space: nowrap; flex-shrink: 0; }
  .dbg-more { flex-shrink: 0; }
  .dbg-stop.is-live { color: var(--danger); }
  .dbg-pane-btn {
    display: inline-flex; align-items: center; gap: 4px; block-size: 22px; padding-inline: 6px;
    border: 1px solid var(--border); border-radius: var(--radius-s);
    background: var(--surface-2); color: var(--text-dim); font-size: var(--fs-xs); white-space: nowrap; cursor: pointer;
  }
  .dbg-pane-btn:hover { background: var(--hover); color: var(--text); }
  .dbg-sep { inline-size: 1px; block-size: 14px; background: var(--border); margin-inline: 2px; flex-shrink: 0; }
  .dbg-empty-wrap { flex: 1; min-block-size: 0; overflow-y: auto; display: flex; align-items: center; justify-content: center; }
  .dbg-full-console, .dbg-console-col { flex: 1; min-block-size: 0; min-inline-size: 0; display: flex; flex-direction: column; }
  .dbg-flex { flex: 1; min-block-size: 0; min-inline-size: 0; display: flex; overflow: hidden; }
  .dbg-quick-repl { display: flex; align-items: center; gap: 6px; padding: 4px 8px; border-block-start: 1px solid var(--separator); background: var(--surface-2); font-size: var(--fs-xs); }
  .dbg-repl-expr { color: var(--text-dim); flex-shrink: 0; }
  .dbg-repl-val { flex: 1; min-inline-size: 0; color: var(--text); }
  .dbg-repl-val.is-err { color: var(--danger); }
  .dbg-repl-form { display: flex; align-items: center; gap: 6px; padding: 4px 8px; border-block-start: 1px solid var(--border); background: var(--surface); flex-shrink: 0; }
  .dbg-inline-input { flex: 1; min-inline-size: 0; block-size: 24px; font-size: var(--fs-xs); }
  @container (max-width: 560px) { .dbg-loc { display: none; } }
  @container (max-width: 460px) { .dbg-subtabs, .dbg-meta { display: none; } }
  /* Stacked (phone) layout, decided by the IDE's own layout flag: 44 px tap
     targets; the row wraps (status + view switch under the controls) rather
     than scrolling sideways. */
  .dbg-root.is-compact .dbg-bar { block-size: auto; max-block-size: none; flex-wrap: wrap; padding-block: 4px; row-gap: 4px; }
  .dbg-root.is-compact .dbg-controls :global(.icon-btn), .dbg-root.is-compact .dbg-more { min-inline-size: 44px; min-block-size: 44px; }
  .dbg-root.is-compact .dbg-subtabs { block-size: 36px; }
  .dbg-root.is-compact .dbg-meta { display: flex; }
  .dbg-root.is-compact .dbg-subtabs { display: inline-flex; }
  .dbg-root.is-compact .dbg-flex { flex-direction: column; overflow-y: auto; }
</style>
