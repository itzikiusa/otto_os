<script lang="ts">
  // Full DAP Debugger panel: fixed-height execution toolbar, Tabbed vs Split
  // layout modes, hideable panes, draggable column resizers, and shared crisp
  // JSON-formatted Debug Console + Evaluate expression bar.
  import { onMount, untrack } from 'svelte';
  import Badge from '../../lib/components/Badge.svelte';
  import EmptyState from '../../lib/components/EmptyState.svelte';
  import Icon from '../../lib/components/Icon.svelte';
  import LoadState from '../../lib/components/LoadState.svelte';
  import Switch from '../../lib/components/Switch.svelte';
  import { announceShown } from '../../lib/announce.svelte';
  import { api } from '../../lib/api/client';
  import { pollWhileVisible } from '../../lib/poll';
  import type { BadgeTone } from '../../lib/status';
  import {
    ide,
    ideBasename,
    type IdeDebugSessionSnapshot,
  } from './ideStore.svelte';
  import { ideAgent } from './store/ideAgentController.svelte';
  import {
    areIdeBreakpointsMuted,
    toggleIdeBreakpointsMuted,
  } from './editor/ideEditorStoreBridge.svelte';
  import IdeBreakpointsModal from './debug/IdeBreakpointsModal.svelte';
  import IdeConsoleView, { type ConsoleLineInput } from './debug/IdeConsoleView.svelte';
  import IdeDebuggerInspector from './debug/IdeDebuggerInspector.svelte';
  import { ideDockPrefs } from './debug/ideDockPrefs.svelte';

  let activeSubTab = $state<'debugger' | 'console'>('debugger');
  let replInput = $state('');
  let replHistory = $state<string[]>([]);
  let historyIdx = $state(-1);
  let evaluating = $state(false);
  let bpModalOpen = $state(false);

  const dbg = $derived(ide.activeDebug);
  const consoleLines = $derived<readonly ConsoleLineInput[]>(
    (dbg?.output ?? []).map((o) => ({ text: o.text, isError: o.category === 'stderr' })),
  );
  const consoleCount = $derived(consoleLines.length + ide.replEntries.length);
  const latestRepl = $derived(
    ide.replEntries.length > 0 ? ide.replEntries[ide.replEntries.length - 1] : null,
  );
  const isPaused = $derived(dbg?.state === 'paused');
  const isRunning = $derived(dbg?.state === 'running' || dbg?.state === 'starting');
  const isLive = $derived(Boolean(dbg && dbg.state !== 'terminated' && dbg.state !== 'error'));

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

  onMount(() => {
    if (ide.activeDebug) return;
    const ctl = new AbortController();
    void api.bg
      .get<{ sessions: IdeDebugSessionSnapshot[] }>('/ide/debug/sessions', ctl.signal)
      .then((list) => {
        if (ide.activeDebug) return;
        const sessions = list.sessions ?? [];
        const live = sessions.find((s) => s.state !== 'terminated') ?? sessions[0] ?? null;
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
          const changed =
            next.state !== cur.state ||
            next.active_frame_id !== cur.active_frame_id ||
            next.paused_file !== cur.paused_file ||
            next.paused_line !== cur.paused_line ||
            (next.stack_frames?.length ?? 0) !== (cur.stack_frames?.length ?? 0) ||
            (next.scopes?.length ?? 0) !== (cur.scopes?.length ?? 0) ||
            (next.output?.length ?? 0) !== (cur.output?.length ?? 0);
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
    try { await ide.evaluateDebug(expr, 'repl', dbg.active_frame_id ?? undefined); }
    finally { evaluating = false; }
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
</script>

{#snippet evalBar(showLatestPreview: boolean)}
  {#if showLatestPreview && latestRepl}
    <div class="dbg-quick-repl">
      <span class="dbg-repl-expr mono">&gt; {latestRepl.expression}:</span>
      <span class="dbg-repl-val mono ellipsis" class:is-err={latestRepl.isError} title={latestRepl.result}>{latestRepl.result}</span>
      <button type="button" class="dbg-pane-btn" onclick={() => (activeSubTab = 'console')} title="Open full Console output">
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

<div class="dbg-root" data-testid="ide-debugger-panel">
  <header class="dbg-bar">
    <div class="dbg-controls" role="toolbar" aria-label="Debugger execution controls">
      {#if !dbg || dbg.state === 'terminated' || dbg.state === 'error'}
        <button
          type="button"
          class="btn small dbg-compact-btn"
          disabled={!ide.hasTarget || ide.debugLoading}
          title={!ide.hasTarget ? 'Select a workspace target to start debugging' : ide.debugLoading ? 'Starting debugger…' : 'Start debugging'}
          aria-keyshortcuts="Control+D"
          onclick={() => void ide.startDebug()}
        ><Icon name="bug" size={12} /><span>{ide.debugLoading ? 'Starting…' : 'Debug'}</span></button>
      {/if}
      <button type="button" class="icon-btn" aria-label="Resume program" title={isPaused ? 'Resume Program (⌘⌥R / F9)' : undefined} aria-keyshortcuts="Meta+Alt+R F9" disabled={!isPaused} onclick={() => void ide.resumeDebug()}><Icon name="play" size={13} /></button>
      <button type="button" class="icon-btn" aria-label="Pause program" title={isRunning ? 'Pause program' : undefined} disabled={!isRunning} onclick={() => void ide.pauseDebug()}><Icon name="pause" size={13} /></button>
      <span class="dbg-sep" aria-hidden="true"></span>
      <button type="button" class="icon-btn" aria-label="Step over" title={isPaused ? 'Step Over (F8 / ⌃⌥↓)' : undefined} aria-keyshortcuts="F8 Control+Alt+ArrowDown" disabled={!isPaused} onclick={() => void ide.stepOver()}><Icon name="stepOver" size={13} /></button>
      <button type="button" class="icon-btn" aria-label="Step into" title={isPaused ? 'Step Into (F7 / ⌃⌥→)' : undefined} aria-keyshortcuts="F7 Control+Alt+ArrowRight" disabled={!isPaused} onclick={() => void ide.stepInto()}><Icon name="stepInto" size={13} /></button>
      <button type="button" class="icon-btn" aria-label="Step out" title={isPaused ? 'Step Out (⇧F8 / ⌃⌥↑)' : undefined} aria-keyshortcuts="Shift+F8 Control+Alt+ArrowUp" disabled={!isPaused} onclick={() => void ide.stepOut()}><Icon name="stepOut" size={13} /></button>
      <button type="button" class="icon-btn" aria-label="Run to cursor" title={isPaused && ide.activePath ? 'Run to Cursor (⌥F9)' : undefined} aria-keyshortcuts="Alt+F9" disabled={!isPaused || !ide.activePath} onclick={() => void ide.runToCursor()}><Icon name="target" size={13} /></button>
      <span class="dbg-sep" aria-hidden="true"></span>
      <button
        type="button"
        class="icon-btn"
        aria-label={areIdeBreakpointsMuted() ? 'Unmute breakpoints' : 'Mute all breakpoints'}
        title={areIdeBreakpointsMuted() ? 'Unmute breakpoints' : 'Mute all breakpoints'}
        aria-pressed={areIdeBreakpointsMuted()}
        onclick={() => toggleIdeBreakpointsMuted()}
        data-testid="ide-debug-mute-bps"
      ><Icon name={areIdeBreakpointsMuted() ? 'shieldOff' : 'shield'} size={13} /></button>
      <button type="button" class="icon-btn" aria-label="Restart debug session" title={isLive || ide.canRestartDebug ? 'Restart debug session' : undefined} disabled={(!isLive && !ide.canRestartDebug) || ide.debugLoading} onclick={() => void ide.restartDebug()} data-testid="ide-debug-restart"><Icon name="refresh" size={13} /></button>
      <button type="button" class="icon-btn" aria-label="Stop debug session" title={isLive ? 'Stop (⌘F2)' : undefined} aria-keyshortcuts="Meta+F2" disabled={!isLive} onclick={() => void ide.stopDebug()}><Icon name="stop" size={13} /></button>
    </div>

    {#if dbg}
      <div class="dbg-meta">
        <Badge tone={statusBadge.tone} label={statusBadge.label} dot live={statusBadge.live} status={dbg.state} />
        <Badge tone="neutral" variant="outline" label={`${dbg.language} · ${dbg.adapter}`} />
        {#if dbg.paused_file && dbg.paused_line}
          <button
            type="button"
            class="btn small ghost mono dbg-compact-btn"
            dir="ltr"
            title={`Jump to paused line ${dbg.paused_file}:${dbg.paused_line}`}
            onclick={() => dbg.paused_file && dbg.paused_line && void ide.openFile(dbg.paused_file, { line: dbg.paused_line, preview: false })}
          >{ideBasename(dbg.paused_file)}:{dbg.paused_line}</button>
        {/if}
      </div>

      <span class="dbg-sep" aria-hidden="true"></span>

      {#if ideDockPrefs.debuggerLayout === 'tabbed'}
        <div class="segmented dbg-subtabs" role="group" aria-label="Debugger sub-views">
          <button type="button" class:active={activeSubTab === 'debugger'} aria-pressed={activeSubTab === 'debugger'} onclick={() => (activeSubTab = 'debugger')} data-testid="ide-debug-subtab-debugger">Debugger</button>
          <button type="button" class:active={activeSubTab === 'console'} aria-pressed={activeSubTab === 'console'} onclick={() => (activeSubTab = 'console')} data-testid="ide-debug-subtab-console">Console{consoleCount > 0 ? ` (${consoleCount})` : ''}</button>
        </div>
        {#if activeSubTab === 'debugger'}
          <button
            type="button"
            class="dbg-pane-btn"
            class:is-on={ideDockPrefs.dbgShowFrames}
            aria-pressed={ideDockPrefs.dbgShowFrames}
            title={ideDockPrefs.dbgShowFrames ? 'Hide Frames column' : 'Show Frames column'}
            onclick={() => ideDockPrefs.toggleFrames()}
            data-testid="ide-debug-toggle-frames"
          >Frames</button>
        {/if}
      {:else}
        <div class="dbg-pane-group" role="group" aria-label="Visible debugger columns">
          <button type="button" class="dbg-pane-btn" class:is-on={ideDockPrefs.dbgShowFrames} aria-pressed={ideDockPrefs.dbgShowFrames} title="Toggle Frames column" onclick={() => ideDockPrefs.toggleFrames()} data-testid="ide-debug-toggle-frames">Frames</button>
          <button type="button" class="dbg-pane-btn" class:is-on={ideDockPrefs.dbgShowVariables} aria-pressed={ideDockPrefs.dbgShowVariables} title="Toggle Variables column" onclick={() => ideDockPrefs.toggleVariables()} data-testid="ide-debug-toggle-variables">Variables</button>
          <button type="button" class="dbg-pane-btn" class:is-on={ideDockPrefs.dbgShowConsole} aria-pressed={ideDockPrefs.dbgShowConsole} title="Toggle Console column" onclick={() => ideDockPrefs.toggleConsolePane()} data-testid="ide-debug-toggle-console">Console</button>
        </div>
      {/if}
    {/if}

    <span class="grow"></span>

    <div class="dbg-right">
      <button
        type="button"
        class="dbg-pane-btn"
        aria-pressed={ideDockPrefs.debuggerLayout === 'split'}
        aria-label={ideDockPrefs.debuggerLayout === 'tabbed' ? 'Switch to split debugger columns' : 'Switch to tabbed debugger and console'}
        title={ideDockPrefs.debuggerLayout === 'tabbed' ? 'Switch to 3-column Split view' : 'Switch to GoLand Tabbed view (Debugger | Console)'}
        onclick={() => ideDockPrefs.toggleDebuggerLayout()}
        data-testid="ide-debug-layout-toggle"
      >
        <Icon name={ideDockPrefs.debuggerLayout === 'tabbed' ? 'columns' : 'layout'} size={12} />
        <span>{ideDockPrefs.debuggerLayout === 'tabbed' ? 'Split' : 'Tabbed'}</span>
      </button>
      <button
        type="button"
        class="btn small ghost dbg-agent-btn dbg-compact-btn"
        disabled={!isPaused || !dbg}
        title={isPaused ? 'Ask AI Agent about paused stack frame and variables' : 'Pause at a breakpoint to ask AI Agent'}
        onclick={() => dbg && ideAgent.attachDebugFrame(dbg, ide.expandedVariables, { openPanel: true })}
        data-testid="ide-debug-ask-agent"
      ><Icon name="assistant" size={12} /><span>Ask Agent</span></button>
      <div class="dbg-exc">
        <Switch checked={ide.breakOnExceptions} onchange={(next) => { ide.breakOnExceptions = next; void ide.syncBreakpointsToSession(); }} label="Break on exceptions" title="Pause on thrown exceptions" />
        <button type="button" tabindex="-1" class="dbg-exc-label" onclick={() => { ide.breakOnExceptions = !ide.breakOnExceptions; void ide.syncBreakpointsToSession(); }}>Exceptions</button>
      </div>
      <button type="button" class="btn small ghost dbg-compact-btn" onclick={() => (bpModalOpen = true)}>
        <Icon name="dot" size={12} /><span>Breakpoints ({ide.allBreakpoints.length})</span>
      </button>
    </div>
  </header>

  {#if !dbg}
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
  {:else}
    {#if dbg.error}
      <div class="dbg-err-banner" use:announceShown={{ text: dbg.error, assertive: true }}>
        <Icon name="warning" size={13} /><span class="ellipsis" title={dbg.error}>{dbg.error}</span>
      </div>
    {/if}

    {#if ideDockPrefs.debuggerLayout === 'tabbed'}
      {#if activeSubTab === 'console'}
        <div class="dbg-full-console" data-testid="ide-debug-console-full">
          <IdeConsoleView
            sessionKey={dbg.id}
            lines={consoleLines}
            replEntries={ide.replEntries}
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
              replEntries={ide.replEntries}
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
  {/if}
</div>

{#if bpModalOpen}
  <IdeBreakpointsModal initialPath={ide.activePath ?? ''} initialLine={ide.cursorInfo.line || 1} onclose={() => (bpModalOpen = false)} />
{/if}

<style>
  .dbg-root { container-type: inline-size; display: flex; flex-direction: column; block-size: 100%; min-block-size: 0; min-inline-size: 0; background: var(--surface); }
  .dbg-bar {
    display: flex; align-items: center; gap: 6px;
    block-size: 32px; min-block-size: 32px; max-block-size: 32px;
    padding-inline: 8px; border-block-end: 1px solid var(--border);
    background: var(--surface); flex-shrink: 0; overflow-x: auto; overflow-y: hidden;
  }
  .dbg-controls, .dbg-meta, .dbg-right, .dbg-pane-group { display: flex; align-items: center; gap: 4px; min-inline-size: 0; flex-shrink: 0; }
  .dbg-compact-btn { block-size: 22px; padding-block: 0; }
  .dbg-subtabs { block-size: 22px; flex-shrink: 0; }
  .dbg-subtabs button { padding-block: 0; padding-inline: 8px; font-size: var(--fs-xs); white-space: nowrap; flex-shrink: 0; }
  .dbg-pane-btn {
    display: inline-flex; align-items: center; gap: 4px; block-size: 22px; padding-inline: 6px;
    border: 1px solid var(--border); border-radius: var(--radius-s);
    background: var(--surface-2); color: var(--text-dim); font-size: var(--fs-xs); white-space: nowrap; cursor: pointer;
  }
  .dbg-pane-btn:hover { background: var(--hover); color: var(--text); }
  .dbg-pane-btn.is-on { border-color: var(--accent-line); background: var(--accent-soft); color: var(--accent-text); }
  .dbg-sep { inline-size: 1px; block-size: 14px; background: var(--border); margin-inline: 2px; flex-shrink: 0; }
  .dbg-agent-btn:not(:disabled) { color: var(--accent-text); }
  .dbg-exc { display: inline-flex; align-items: center; gap: 4px; padding-inline: 2px; }
  .dbg-exc-label { border: 0; background: transparent; padding: 0; font-size: var(--fs-xs); color: var(--text-dim); cursor: pointer; }
  .dbg-empty-wrap { flex: 1; min-block-size: 0; overflow-y: auto; display: flex; align-items: center; justify-content: center; }
  .dbg-err-banner { display: flex; align-items: center; gap: 8px; padding: 4px 12px; font-size: var(--fs-xs); color: var(--danger); background: var(--danger-soft); border-block-end: 1px solid var(--border); flex-shrink: 0; }
  .dbg-full-console, .dbg-console-col { flex: 1; min-block-size: 0; min-inline-size: 0; display: flex; flex-direction: column; }
  .dbg-flex { flex: 1; min-block-size: 0; min-inline-size: 0; display: flex; overflow: hidden; }
  .dbg-quick-repl { display: flex; align-items: center; gap: 6px; padding: 4px 8px; border-block-start: 1px solid var(--separator); background: var(--surface-2); font-size: var(--fs-xs); }
  .dbg-repl-expr { color: var(--text-dim); flex-shrink: 0; }
  .dbg-repl-val { flex: 1; min-inline-size: 0; color: var(--text); }
  .dbg-repl-val.is-err { color: var(--danger); }
  .dbg-repl-form { display: flex; align-items: center; gap: 6px; padding: 4px 8px; border-block-start: 1px solid var(--border); background: var(--surface); flex-shrink: 0; }
  .dbg-inline-input { flex: 1; min-inline-size: 0; block-size: 24px; font-size: var(--fs-xs); }
  @media (max-width: 640px) { .dbg-flex { flex-direction: column; overflow-y: auto; } }
</style>
