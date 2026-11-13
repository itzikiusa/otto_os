<script lang="ts">
  // Full DAP Debugger panel: execution toolbar, Breakpoints/Conditions/Logpoints
  // modal, and 3-column desktop / stacked mobile inspector (Threads & Call Stack,
  // Scopes/Variables + Watches, and Debug Console + Evaluate REPL).
  import { onMount, untrack } from 'svelte';
  import Badge from '../../lib/components/Badge.svelte';
  import EmptyState from '../../lib/components/EmptyState.svelte';
  import Icon from '../../lib/components/Icon.svelte';
  import LoadState from '../../lib/components/LoadState.svelte';
  import Modal from '../../lib/components/Modal.svelte';
  import Switch from '../../lib/components/Switch.svelte';
  import { announceShown } from '../../lib/announce.svelte';
  import { api } from '../../lib/api/client';
  import { copyWithToast } from '../../lib/clipboard';
  import { scrollBehavior } from '../../lib/motion';
  import { pollWhileVisible } from '../../lib/poll';
  import type { BadgeTone } from '../../lib/status';
  import {
    ide,
    ideBasename,
    type IdeBreakpoint,
    type IdeDebugSessionSnapshot,
    type IdeScope,
    type IdeStackFrame,
    type IdeVariable,
  } from './ideStore.svelte';
  import { ideAgent } from './store/ideAgentController.svelte';

  const LOG_CAP = 500;

  let openScopes = $state<Record<string, boolean>>({});
  let openRefs = $state<Record<number, boolean>>({});
  let loadingRefs = $state<Record<number, boolean>>({});
  let watchDraft = $state('');
  let replInput = $state('');
  let replHistory = $state<string[]>([]);
  let historyIdx = $state(-1);
  let evaluating = $state(false);
  let bpModalOpen = $state(false);
  let newBpPath = $state('');
  let newBpLine = $state('1');
  let showAllOutput = $state(false);
  let consoleEl = $state<HTMLDivElement>();
  let stickBottom = $state(true);

  const dbg = $derived(ide.activeDebug);
  const allOutput = $derived(dbg?.output ?? []);
  const hiddenOutCount = $derived(!showAllOutput && allOutput.length > LOG_CAP ? allOutput.length - LOG_CAP : 0);
  const visibleOutput = $derived(hiddenOutCount > 0 ? allOutput.slice(-LOG_CAP) : allOutput);
  const isPaused = $derived(dbg?.state === 'paused');
  const isRunning = $derived(dbg?.state === 'running' || dbg?.state === 'starting');
  const isLive = $derived(!!dbg && dbg.state !== 'terminated' && dbg.state !== 'error');

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

  // eslint-disable-next-line no-control-regex
  const ANSI_RE = /\x1b\[[0-9;]*[a-zA-Z]|\x1b\][^\x07]*\x07/g;
  const cleanAnsi = (raw: string): string => raw.replace(ANSI_RE, '');
  const scopeKey = (s: IdeScope): string => `${s.name}:${s.variables_reference}`;

  // One-shot discovery on mount if a debug session was already started via CLI/API;
  // never polls while idle (`!dbg`).
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
      .catch(() => { /* no active debug session */ });
    return () => ctl.abort();
  });

  // Poll session state ONLY while an active (non-terminated, non-errored) session exists.
  $effect(() => {
    if (!dbg || dbg.state === 'terminated' || dbg.state === 'error') return;
    const sessionId = dbg.id;
    const poller = pollWhileVisible(async (signal) => {
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
    }, { ms: 900, floorMs: 500, immediate: false });
    return () => poller.stop();
  });

  // Auto-open and load the top scope (e.g. Locals) when paused at a frame.
  const autoFetchedScopes = new Set<string>();
  $effect(() => {
    const cur = dbg;
    if (!cur || cur.state !== 'paused' || !cur.scopes?.length) return;
    const first = cur.scopes[0];
    if (!first || first.variables_reference <= 0) return;
    const key = `${cur.id}:${cur.active_frame_id ?? 0}:${first.name}:${first.variables_reference}`;
    if (autoFetchedScopes.has(key)) return;
    autoFetchedScopes.add(key);
    openScopes = { ...openScopes, [scopeKey(first)]: true };
    if (!ide.expandedVariables[first.variables_reference]) void loadRef(first.variables_reference);
  });

  // Reset log cap toggle when a new debug session starts.
  let lastDebugId = '';
  $effect(() => {
    const id = dbg?.id ?? '';
    if (id && id !== lastDebugId) { lastDebugId = id; showAllOutput = false; stickBottom = true; }
  });

  // Keep console scrolled to bottom when new output or REPL entries arrive.
  $effect(() => {
    void dbg?.output?.length;
    void ide.replEntries.length;
    const el = consoleEl;
    if (!el || !stickBottom) return;
    queueMicrotask(() => { el.scrollTop = el.scrollHeight; });
  });

  async function loadRef(ref: number): Promise<void> {
    if (ref <= 0 || loadingRefs[ref]) return;
    loadingRefs = { ...loadingRefs, [ref]: true };
    try { await ide.expandVariables(ref); }
    finally { loadingRefs = { ...loadingRefs, [ref]: false }; }
  }

  function toggleScope(s: IdeScope): void {
    const k = scopeKey(s);
    const next = !openScopes[k];
    openScopes = { ...openScopes, [k]: next };
    if (next && s.variables_reference > 0 && !ide.expandedVariables[s.variables_reference]) {
      void loadRef(s.variables_reference);
    }
  }

  function toggleVar(v: IdeVariable): void {
    const ref = v.variables_reference;
    if (ref <= 0) return;
    const next = !openRefs[ref];
    openRefs = { ...openRefs, [ref]: next };
    if (next && !ide.expandedVariables[ref]) void loadRef(ref);
  }

  function onSelectFrame(frame: IdeStackFrame): void {
    void ide.selectFrame(frame.id, dbg?.active_thread_id ?? undefined);
    if (frame.path) void ide.openFile(frame.path, { line: frame.line, col: frame.column ?? 1, preview: true });
  }

  function onThreadChange(e: Event): void {
    const id = Number((e.currentTarget as HTMLSelectElement).value);
    const firstFrame = dbg?.stack_frames?.[0]?.id;
    if (Number.isFinite(id) && firstFrame !== undefined) void ide.selectFrame(firstFrame, id);
  }

  function onAddWatch(e: SubmitEvent): void {
    e.preventDefault();
    const expr = watchDraft.trim();
    if (!expr) return;
    ide.addWatch(expr);
    watchDraft = '';
  }

  async function onEvalSubmit(e: SubmitEvent): Promise<void> {
    e.preventDefault();
    const expr = replInput.trim();
    if (!expr || !dbg || evaluating) return;
    evaluating = true;
    replHistory = [expr, ...replHistory.filter((h) => h !== expr)].slice(0, 50);
    historyIdx = -1;
    replInput = '';
    stickBottom = true;
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

  function onConsoleScroll(): void {
    if (consoleEl) stickBottom = consoleEl.scrollHeight - consoleEl.scrollTop - consoleEl.clientHeight < 24;
  }

  function jumpConsoleBottom(): void {
    if (!consoleEl) return;
    stickBottom = true;
    consoleEl.scrollTo({ top: consoleEl.scrollHeight, behavior: scrollBehavior() });
  }

  function copyConsoleAll(): void {
    const outLines = (dbg?.output ?? []).map((o) => cleanAnsi(o.text).trimEnd());
    const replLines = ide.replEntries.map((r) => `> ${r.expression}\n${r.result}`);
    const combined = [...outLines, ...replLines].filter(Boolean).join('\n');
    if (combined) void copyWithToast(combined, 'debug console');
  }

  function addManualBreakpoint(e: SubmitEvent): void {
    e.preventDefault();
    const path = (newBpPath.trim() || ide.activePath || '').trim();
    const line = Math.max(1, Math.floor(Number(newBpLine) || 1));
    if (!path) return;
    ide.updateBreakpoint(path, line, { enabled: true });
    newBpPath = '';
    newBpLine = '1';
  }

  const updateBpField = (bp: IdeBreakpoint, patch: Partial<IdeBreakpoint>): void =>
    ide.updateBreakpoint(bp.path, bp.line, patch);
</script>

{#snippet variableRows(vars: IdeVariable[], depth: number)}
  {#each vars as v (`${depth}:${v.name}:${v.variables_reference}`)}
    {@const expandable = v.variables_reference > 0}
    {@const expanded = !!openRefs[v.variables_reference]}
    {@const children = ide.expandedVariables[v.variables_reference] ?? []}
    <div class="dbg-var-row reveal-host" style:padding-inline-start={`${12 + depth * 14}px`}>
      {#if expandable}
        <button
          type="button"
          class="icon-btn dbg-twisty"
          aria-expanded={expanded}
          aria-label={expanded ? `Collapse ${v.name}` : `Expand ${v.name}`}
          title={expanded ? `Collapse ${v.name}` : `Expand ${v.name}`}
          onclick={() => toggleVar(v)}
        ><Icon name={expanded ? 'chevronDown' : 'chevronRight'} size={12} /></button>
      {:else}
        <span class="dbg-twisty-spacer" aria-hidden="true"></span>
      {/if}
      <span class="dbg-var-name mono" dir="ltr">{v.name}</span>
      <span class="dbg-var-eq dim">=</span>
      <span class="dbg-var-val mono ellipsis" dir="ltr" title={v.value}>{v.value}</span>
      {#if v.var_type}<span class="dbg-var-type mono ellipsis" dir="ltr" title={v.var_type}>{v.var_type}</span>{/if}
      <button
        type="button"
        class="icon-btn reveal-on-hover"
        aria-label={`Copy value of ${v.name}`}
        title={`Copy value of ${v.name}`}
        onclick={() => void copyWithToast(v.value, v.name)}
      ><Icon name="copy" size={12} /></button>
    </div>
    {#if expandable && expanded}
      {#if loadingRefs[v.variables_reference] && children.length === 0}
        <div class="dbg-var-empty dim" style:padding-inline-start={`${26 + depth * 14}px`}>
          <span class="spinner" style="--spinner-size: 12px" aria-hidden="true"></span>
          <span>Inspecting fields…</span>
        </div>
      {:else if children.length === 0}
        <div class="dbg-var-empty dim" style:padding-inline-start={`${26 + depth * 14}px`}>Empty</div>
      {:else}
        {@render variableRows(children, depth + 1)}
      {/if}
    {/if}
  {/each}
{/snippet}

<div class="dbg-root" data-testid="ide-debugger-panel">
  <header class="dbg-bar">
    <div class="dbg-controls" role="toolbar" aria-label="Debugger execution controls">
      {#if !dbg || dbg.state === 'terminated' || dbg.state === 'error'}
        <button
          type="button"
          class="btn small"
          disabled={!ide.hasTarget || ide.debugLoading}
          title={!ide.hasTarget ? 'Select a workspace target to start debugging' : ide.debugLoading ? 'Starting debugger…' : 'Start debugging'}
          aria-keyshortcuts="Control+Shift+D"
          onclick={() => void ide.startDebug()}
        >
          <Icon name="bug" size={12} /><span>{ide.debugLoading ? 'Starting…' : 'Debug'}</span>
        </button>
      {/if}
      <button type="button" class="icon-btn" aria-label="Resume program" title="Resume program" aria-keyshortcuts="Alt+Meta+R" disabled={!isPaused} onclick={() => void ide.resumeDebug()}>
        <Icon name="play" size={13} />
      </button>
      <button type="button" class="icon-btn" aria-label="Pause program" title="Pause program" disabled={!isRunning} onclick={() => void ide.pauseDebug()}>
        <Icon name="pause" size={13} />
      </button>
      <span class="dbg-sep" aria-hidden="true"></span>
      <button type="button" class="icon-btn" aria-label="Step over" title="Step over" aria-keyshortcuts="F8" disabled={!isPaused} onclick={() => void ide.stepOver()}>
        <Icon name="stepOver" size={13} />
      </button>
      <button type="button" class="icon-btn" aria-label="Step into" title="Step into" aria-keyshortcuts="F7" disabled={!isPaused} onclick={() => void ide.stepInto()}>
        <Icon name="stepInto" size={13} />
      </button>
      <button type="button" class="icon-btn" aria-label="Step out" title="Step out" aria-keyshortcuts="Shift+F8" disabled={!isPaused} onclick={() => void ide.stepOut()}>
        <Icon name="stepOut" size={13} />
      </button>
      <button type="button" class="icon-btn" aria-label="Run to cursor" title="Run to cursor" aria-keyshortcuts="Alt+F9" disabled={!isPaused || !ide.activePath} onclick={() => void ide.runToCursor()}>
        <Icon name="target" size={13} />
      </button>
      <span class="dbg-sep" aria-hidden="true"></span>
      <button type="button" class="icon-btn" aria-label="Restart debug session" title="Restart debug session" disabled={!isLive} onclick={() => void ide.restartDebug()}>
        <Icon name="refresh" size={13} />
      </button>
      <button type="button" class="icon-btn" aria-label="Stop debug session" title="Stop debug session" disabled={!isLive} onclick={() => void ide.stopDebug()}>
        <Icon name="stop" size={13} />
      </button>
    </div>

    {#if dbg}
      <div class="dbg-meta">
        <Badge tone={statusBadge.tone} label={statusBadge.label} dot live={statusBadge.live} status={dbg.state} />
        <Badge tone="neutral" variant="outline" label={`${dbg.language} · ${dbg.adapter}`} />
        {#if dbg.paused_file && dbg.paused_line}
          <button
            type="button"
            class="btn small ghost mono"
            dir="ltr"
            title={`Jump to paused line ${dbg.paused_file}:${dbg.paused_line}`}
            onclick={() => dbg.paused_file && dbg.paused_line && void ide.openFile(dbg.paused_file, { line: dbg.paused_line, preview: true })}
          >{ideBasename(dbg.paused_file)}:{dbg.paused_line}</button>
        {/if}
      </div>
    {/if}

    <span class="grow"></span>

    <div class="dbg-right">
      <button
        type="button"
        class="btn small ghost dbg-agent-btn"
        disabled={!isPaused || !dbg}
        title={isPaused ? 'Ask AI Agent about paused stack frame and variables' : 'Pause at a breakpoint to ask AI Agent'}
        onclick={() => dbg && ideAgent.attachDebugFrame(dbg, ide.expandedVariables, { openPanel: true })}
        data-testid="ide-debug-ask-agent"
      ><Icon name="assistant" size={12} /><span>Ask Agent</span></button>
      <div class="dbg-exc">
        <Switch
          checked={ide.breakOnExceptions}
          onchange={(next) => { ide.breakOnExceptions = next; void ide.syncBreakpointsToSession(); }}
          label="Break on exceptions"
          title="Pause on thrown exceptions"
        />
        <button
          type="button"
          tabindex="-1"
          class="dbg-exc-label"
          onclick={() => { ide.breakOnExceptions = !ide.breakOnExceptions; void ide.syncBreakpointsToSession(); }}
        >Exceptions</button>
      </div>
      <button
        type="button"
        class="btn small ghost"
        onclick={() => { newBpPath = ide.activePath ?? ''; newBpLine = String(ide.cursorInfo.line || 1); bpModalOpen = true; }}
      ><Icon name="dot" size={12} /><span>Breakpoints ({ide.allBreakpoints.length})</span></button>
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
        <Icon name="warning" size={13} />
        <span class="ellipsis" title={dbg.error}>{dbg.error}</span>
      </div>
    {/if}

    <div class="dbg-grid">
      <!-- Column 1: Threads & Call Stack -->
      <section class="dbg-col" aria-label="Threads and call stack">
        <header class="dbg-col-head">
          <h2 class="dbg-col-title">Frames</h2>
          {#if dbg.threads.length > 0}
            <select class="input dbg-thread-select" dir="ltr" aria-label="Active thread" value={dbg.active_thread_id ?? dbg.threads[0]?.id} onchange={onThreadChange}>
              {#each dbg.threads as th (th.id)}<option value={th.id}>#{th.id} {th.name}</option>{/each}
            </select>
          {/if}
        </header>
        <div class="dbg-scroll">
          {#if dbg.stack_frames.length === 0}
            <p class="dbg-muted">{isPaused ? 'No stack frames reported.' : 'Pause execution or hit a breakpoint to inspect stack frames.'}</p>
          {:else}
            {#each dbg.stack_frames as frame (frame.id)}
              <button
                type="button"
                class="dbg-frame"
                class:is-selected={dbg.active_frame_id === frame.id}
                aria-current={dbg.active_frame_id === frame.id ? 'true' : undefined}
                onclick={() => onSelectFrame(frame)}
              >
                <span class="dbg-frame-fn mono ellipsis" dir="ltr" title={frame.name}>{frame.name}</span>
                {#if frame.path}
                  <span class="dbg-frame-loc mono" dir="ltr" title={`${frame.path}:${frame.line}`}>{ideBasename(frame.path)}:{frame.line}</span>
                {/if}
              </button>
            {/each}
          {/if}
        </div>
      </section>

      <!-- Column 2: Variables & Watches -->
      <section class="dbg-col" aria-label="Variables and watches">
        <header class="dbg-col-head">
          <h2 class="dbg-col-title">Variables &amp; watches</h2>
          {#if ide.watches.length > 0}
            <button type="button" class="icon-btn" aria-label="Refresh watches" title="Refresh watches" disabled={!isPaused} onclick={() => void ide.refreshWatches()}>
              <Icon name="refresh" size={12} />
            </button>
          {/if}
        </header>
        <div class="dbg-scroll">
          {#if dbg.scopes.length === 0}
            <p class="dbg-muted">{isPaused ? 'No variable scopes in this frame.' : 'Variables populate when paused at a breakpoint.'}</p>
          {:else}
            {#each dbg.scopes as scope (scopeKey(scope))}
              {@const sKey = scopeKey(scope)}
              {@const isOpen = !!openScopes[sKey]}
              {@const vars = ide.expandedVariables[scope.variables_reference] ?? []}
              <div class="dbg-scope">
                <button type="button" class="dbg-scope-head" aria-expanded={isOpen} onclick={() => toggleScope(scope)}>
                  <Icon name={isOpen ? 'chevronDown' : 'chevronRight'} size={12} />
                  <span class="dbg-scope-name">{scope.name}</span>
                  {#if scope.expensive}<Badge tone="neutral" variant="outline" label="expensive" />{/if}
                </button>
                {#if isOpen}
                  {#if loadingRefs[scope.variables_reference] && vars.length === 0}
                    <div class="dbg-var-empty dim">
                      <span class="spinner" style="--spinner-size: 12px" aria-hidden="true"></span><span>Inspecting scope…</span>
                    </div>
                  {:else if vars.length === 0}
                    <div class="dbg-var-empty dim">No variables in {scope.name}</div>
                  {:else}
                    {@render variableRows(vars, 0)}
                  {/if}
                {/if}
              </div>
            {/each}
          {/if}

          <div class="dbg-watches">
            <div class="dbg-subhead">Watches</div>
            <form class="dbg-inline-form" onsubmit={onAddWatch}>
              <input class="input dbg-inline-input" type="text" dir="ltr" aria-label="Add watch expression" placeholder="Add watch expression…" bind:value={watchDraft} />
              <button type="submit" class="btn small" disabled={!watchDraft.trim()}><Icon name="plus" size={12} /><span>Watch</span></button>
            </form>
            {#each ide.watches as w (w.id)}
              <div class="dbg-var-row reveal-host">
                <span class="dbg-var-name mono" dir="ltr">{w.expression}</span>
                <span class="dbg-var-eq dim">=</span>
                {#if w.error}
                  <span class="dbg-watch-err mono ellipsis" dir="ltr" title={w.error}>{w.error}</span>
                {:else}
                  <span class="dbg-var-val mono ellipsis" dir="ltr" title={w.value}>{w.value}</span>
                {/if}
                {#if w.var_type}<span class="dbg-var-type mono ellipsis" dir="ltr">{w.var_type}</span>{/if}
                <button type="button" class="icon-btn reveal-on-hover" aria-label={`Copy watch ${w.expression}`} title={`Copy watch ${w.expression}`} onclick={() => void copyWithToast(w.error || w.value, w.expression)}>
                  <Icon name="copy" size={12} />
                </button>
                <button type="button" class="icon-btn reveal-on-hover" aria-label={`Remove watch ${w.expression}`} title={`Remove watch ${w.expression}`} onclick={() => ide.removeWatch(w.id)}>
                  <Icon name="trash" size={12} />
                </button>
              </div>
            {/each}
          </div>
        </div>
      </section>

      <!-- Column 3: Debug Console & Evaluate REPL -->
      <section class="dbg-col dbg-console-col" aria-label="Debug console and evaluate">
        <header class="dbg-col-head">
          <h2 class="dbg-col-title">Console &amp; evaluate</h2>
          <div class="dbg-head-actions">
            <button type="button" class="icon-btn" aria-label="Copy console output" title="Copy console output" disabled={allOutput.length === 0 && ide.replEntries.length === 0} onclick={copyConsoleAll}>
              <Icon name="copy" size={12} />
            </button>
            <button type="button" class="icon-btn" aria-label="Clear evaluate history" title="Clear evaluate history" disabled={ide.replEntries.length === 0} onclick={() => ide.clearRepl()}>
              <Icon name="trash" size={12} />
            </button>
          </div>
        </header>
        <div class="dbg-console-wrap">
          <div class="dbg-console" dir="ltr" bind:this={consoleEl} onscroll={onConsoleScroll}>
            {#if allOutput.length === 0 && ide.replEntries.length === 0}
              <p class="dbg-muted">Program output and evaluated expressions appear here.</p>
            {/if}
            {#if hiddenOutCount > 0}
              <div class="dbg-cap-bar">
                <button type="button" class="btn small ghost" onclick={() => (showAllOutput = true)}>
                  Show all {allOutput.length} lines ({hiddenOutCount} earlier hidden)
                </button>
              </div>
            {/if}
            {#each visibleOutput as out, idx (`out-${hiddenOutCount + idx}`)}
              <div class="dbg-out-line mono" class:is-err={out.category === 'stderr'}>{cleanAnsi(out.text)}</div>
            {/each}
            {#each ide.replEntries as entry (entry.id)}
              <div class="dbg-repl-item reveal-host">
                <div class="dbg-repl-expr mono">&gt; {entry.expression}</div>
                <div class="dbg-repl-res mono" class:is-err={entry.isError}>
                  <span class="dbg-repl-val">{entry.result}</span>
                  {#if entry.var_type}<span class="dbg-var-type">{entry.var_type}</span>{/if}
                  <button type="button" class="icon-btn reveal-on-hover" aria-label={`Copy result for ${entry.expression}`} title={`Copy result for ${entry.expression}`} onclick={() => void copyWithToast(entry.result, entry.expression)}>
                    <Icon name="copy" size={12} />
                  </button>
                </div>
              </div>
            {/each}
          </div>
          {#if !stickBottom && (allOutput.length > 0 || ide.replEntries.length > 0)}
            <div class="dbg-tail-cta">
              <button type="button" class="btn small" onclick={jumpConsoleBottom}>
                <Icon name="arrowDown" size={12} /><span>Follow output</span>
              </button>
            </div>
          {/if}
        </div>
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
      </section>
    </div>
  {/if}
</div>

{#if bpModalOpen}
  <Modal title="Breakpoints and conditions" width={600} onclose={() => (bpModalOpen = false)}>
    <form class="dbg-bp-add" onsubmit={addManualBreakpoint}>
      <input class="input dbg-inline-input mono" type="text" dir="ltr" aria-label="Breakpoint file path" placeholder={ide.activePath || 'File path…'} bind:value={newBpPath} />
      <input class="input dbg-bp-line mono" type="number" min="1" dir="ltr" aria-label="Breakpoint line number" bind:value={newBpLine} />
      <button type="submit" class="btn small" disabled={!(newBpPath.trim() || ide.activePath)}><Icon name="plus" size={12} /><span>Add</span></button>
    </form>

    {#if ide.allBreakpoints.length === 0}
      <p class="dbg-muted">No breakpoints configured yet. Click any line gutter in the editor or add one above.</p>
    {:else}
      <div class="dbg-bp-list">
        {#each ide.allBreakpoints as bp (`${bp.path}:${bp.line}`)}
          <div class="dbg-bp-card">
            <div class="dbg-bp-top">
              <Switch checked={bp.enabled} onchange={(enabled) => updateBpField(bp, { enabled })} label={`Enable breakpoint at ${ideBasename(bp.path)} line ${bp.line}`} />
              <button type="button" class="btn small ghost mono" dir="ltr" onclick={() => { bpModalOpen = false; void ide.openFile(bp.path, { line: bp.line }); }}>
                {ide.relativePath(bp.path)}:{bp.line}
              </button>
              <span class="grow"></span>
              <button type="button" class="icon-btn" aria-label={`Remove breakpoint at ${ideBasename(bp.path)}:${bp.line}`} title={`Remove breakpoint at ${ideBasename(bp.path)}:${bp.line}`} onclick={() => ide.removeBreakpoint(bp.path, bp.line)}>
                <Icon name="trash" size={13} />
              </button>
            </div>
            <div class="dbg-bp-fields">
              <label class="dbg-bp-field">
                <span>Condition</span>
                <input class="input mono" type="text" dir="ltr" placeholder="e.g. x == 20 && ready" value={bp.condition ?? ''} onchange={(e) => updateBpField(bp, { condition: (e.currentTarget as HTMLInputElement).value.trim() || null })} />
              </label>
              <label class="dbg-bp-field">
                <span>Hit condition</span>
                <input class="input mono" type="text" dir="ltr" placeholder="e.g. >= 5" value={bp.hit_condition ?? ''} onchange={(e) => updateBpField(bp, { hit_condition: (e.currentTarget as HTMLInputElement).value.trim() || null })} />
              </label>
              <label class="dbg-bp-field dbg-bp-wide">
                <span>Logpoint message (logs without pausing)</span>
                <input class="input mono" type="text" dir="ltr" placeholder="e.g. user={'{'}id{'}'} total={'{'}sum{'}'}" value={bp.log_message ?? ''} onchange={(e) => updateBpField(bp, { log_message: (e.currentTarget as HTMLInputElement).value.trim() || null })} />
              </label>
            </div>
          </div>
        {/each}
      </div>
    {/if}

    {#snippet footer()}
      <button type="button" class="btn primary" onclick={() => (bpModalOpen = false)}>Done</button>
    {/snippet}
  </Modal>
{/if}

<style>
  .dbg-root { container-type: inline-size; display: flex; flex-direction: column; block-size: 100%; min-block-size: 0; min-inline-size: 0; background: var(--surface); }
  .dbg-bar { display: flex; flex-wrap: wrap; align-items: center; gap: 8px; padding: 4px 10px; border-block-end: 1px solid var(--border); background: var(--surface); flex-shrink: 0; }
  .dbg-controls, .dbg-meta, .dbg-right, .dbg-head-actions { display: flex; align-items: center; gap: 4px; min-inline-size: 0; }
  .dbg-meta { flex-wrap: wrap; gap: 6px; }
  .dbg-sep { inline-size: 1px; block-size: 14px; background: var(--border); margin-inline: 2px; }
  .dbg-agent-btn:not(:disabled) { color: var(--accent-text); }
  .dbg-exc { display: inline-flex; align-items: center; gap: 6px; padding-inline: 4px; }
  .dbg-exc-label { border: 0; background: transparent; padding: 0; font-size: var(--fs-xs); color: var(--text-dim); cursor: pointer; }
  .dbg-empty-wrap { flex: 1; min-block-size: 0; overflow-y: auto; display: flex; align-items: center; justify-content: center; }
  .dbg-err-banner { display: flex; align-items: center; gap: 8px; padding: 4px 12px; font-size: var(--fs-xs); color: var(--danger); background: var(--danger-soft); border-block-end: 1px solid var(--border); flex-shrink: 0; }
  .dbg-grid { flex: 1; min-block-size: 0; min-inline-size: 0; display: grid; grid-template-columns: minmax(180px, 0.9fr) minmax(220px, 1.25fr) minmax(220px, 1.25fr); }
  .dbg-col { display: flex; flex-direction: column; min-block-size: 0; min-inline-size: 0; border-inline-end: 1px solid var(--border); }
  .dbg-col:last-child { border-inline-end: 0; }
  .dbg-col-head { display: flex; align-items: center; justify-content: space-between; gap: 8px; min-block-size: 28px; padding: 2px 10px; border-block-end: 1px solid var(--border); background: var(--surface-2); flex-shrink: 0; }
  .dbg-col-title, .dbg-subhead { margin: 0; font-size: var(--fs-xs); font-weight: 600; color: var(--text-dim); text-transform: uppercase; letter-spacing: 0.06em; }
  .dbg-subhead { padding: 4px 10px; }
  .dbg-thread-select { block-size: 22px; font-size: var(--fs-xs); max-inline-size: 160px; }
  .dbg-scroll { flex: 1; min-block-size: 0; overflow-y: auto; padding-block: 4px; }
  .dbg-muted { margin: 0; padding: 12px; font-size: var(--fs-xs); color: var(--text-dim); }
  .dbg-frame {
    display: flex; align-items: center; justify-content: space-between; gap: 8px; inline-size: 100%;
    padding: 4px 10px; border: 0; background: transparent; color: var(--text); text-align: start; cursor: pointer; font-size: var(--fs-xs);
  }
  .dbg-frame:hover { background: var(--hover); }
  .dbg-frame.is-selected { background: var(--accent-soft); color: var(--accent-text); }
  .dbg-frame-fn { flex: 1; font-size: var(--fs-xs); }
  .dbg-frame-loc { flex-shrink: 0; font-size: var(--fs-xs); color: var(--text-dim); font-variant-numeric: tabular-nums; }
  .dbg-scope { border-block-end: 1px solid var(--separator); }
  .dbg-scope-head {
    display: flex; align-items: center; gap: 6px; inline-size: 100%; padding: 4px 8px;
    border: 0; background: transparent; color: var(--text); font-size: var(--fs-s); font-weight: 500; text-align: start; cursor: pointer;
  }
  .dbg-scope-head:hover { background: var(--hover); }
  .dbg-scope-name { flex: 1; }
  .dbg-var-row { display: flex; align-items: center; gap: 4px; min-block-size: 24px; padding-block: 2px; padding-inline: 12px 6px; font-size: var(--fs-xs); }
  .dbg-var-row:hover { background: var(--hover); }
  .dbg-twisty { inline-size: 18px; block-size: 18px; }
  .dbg-twisty-spacer { inline-size: 18px; flex-shrink: 0; }
  .dbg-var-name { color: var(--accent-text); font-size: var(--fs-xs); flex-shrink: 0; }
  .dbg-var-eq { font-size: var(--fs-xs); flex-shrink: 0; }
  .dbg-var-val { flex: 1; color: var(--text); font-size: var(--fs-xs); }
  .dbg-var-type { color: var(--text-dim); font-size: var(--fs-xs); max-inline-size: 110px; }
  .dbg-var-empty { display: flex; align-items: center; gap: 6px; padding: 4px 24px; font-size: var(--fs-xs); }
  .dbg-watches { padding-block-start: 6px; }
  .dbg-inline-form { display: flex; align-items: center; gap: 6px; padding: 4px 8px; }
  .dbg-inline-input { flex: 1; min-inline-size: 0; block-size: 24px; font-size: var(--fs-xs); }
  .dbg-watch-err { flex: 1; color: var(--danger); font-size: var(--fs-xs); }
  .dbg-console-wrap { position: relative; flex: 1; min-block-size: 0; display: flex; flex-direction: column; }
  .dbg-console { flex: 1; min-block-size: 0; overflow-y: auto; padding: 8px; background: var(--term-bg); font-size: var(--fs-xs); }
  .dbg-cap-bar { padding-block-end: 6px; }
  .dbg-tail-cta { position: absolute; inset-block-end: 8px; inset-inline-end: 12px; z-index: var(--z-sticky); }
  .dbg-out-line { white-space: pre-wrap; overflow-wrap: anywhere; font-size: var(--fs-xs); line-height: 1.45; }
  .dbg-out-line.is-err, .dbg-repl-res.is-err { color: var(--danger); }
  .dbg-repl-item { padding-block: 4px; border-block-end: 1px solid var(--separator); }
  .dbg-repl-expr { color: var(--text-dim); font-size: var(--fs-xs); overflow-wrap: anywhere; }
  .dbg-repl-res { display: flex; align-items: center; gap: 6px; color: var(--text); font-size: var(--fs-xs); overflow-wrap: anywhere; }
  .dbg-repl-val { flex: 1; min-inline-size: 0; white-space: pre-wrap; overflow-wrap: anywhere; }
  .dbg-repl-form { display: flex; align-items: center; gap: 6px; padding: 6px 8px; border-block-start: 1px solid var(--border); background: var(--surface); flex-shrink: 0; }
  .dbg-bp-add { display: flex; align-items: center; gap: 6px; margin-block-end: 12px; }
  .dbg-bp-line { inline-size: 76px; block-size: 24px; font-size: var(--fs-xs); }
  .dbg-bp-list { display: flex; flex-direction: column; gap: 8px; }
  .dbg-bp-card { padding: 10px; border: 1px solid var(--border); border-radius: var(--radius-m); background: var(--surface-2); }
  .dbg-bp-top { display: flex; align-items: center; gap: 8px; margin-block-end: 8px; }
  .dbg-bp-fields { display: grid; grid-template-columns: 1fr 1fr; gap: 8px; }
  .dbg-bp-field { display: flex; flex-direction: column; gap: 4px; font-size: var(--fs-xs); color: var(--text-dim); }
  .dbg-bp-wide { grid-column: 1 / -1; }
  @container (max-width: 760px) {
    .dbg-grid { grid-template-columns: 1fr; overflow-y: auto; }
    .dbg-col { border-inline-end: 0; border-block-end: 1px solid var(--border); min-block-size: 160px; }
  }
  @media (max-width: 1024px) {
    .dbg-grid { grid-template-columns: 1fr; overflow-y: auto; }
    .dbg-col { border-inline-end: 0; border-block-end: 1px solid var(--border); min-block-size: 160px; }
    .dbg-bp-fields { grid-template-columns: 1fr; }
  }
</style>
