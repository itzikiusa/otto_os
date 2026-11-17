<script lang="ts">
  // Frames & Threads + Variables & Watches inspector columns for `IdeDebuggerPanel`.
  // Variables keep their expansion across steps (open state keyed by path and
  // re-applied after every stop) and mark values that changed since the last stop.
  import { untrack, type Snippet } from 'svelte';
  import Badge from '../../../lib/components/Badge.svelte';
  import Icon from '../../../lib/components/Icon.svelte';
  import PaneDivider from '../../../lib/components/PaneDivider.svelte';
  import { copyWithToast } from '../../../lib/clipboard';
  import { loadPaneWidth } from '../../../lib/paneResizer';
  import { pushIdeNavLocation } from '../editor/ideEditorStoreBridge.svelte';
  import { ideLayout } from '../shell/ideLayout.svelte';
  import {
    ide,
    ideBasename,
    type IdeDebugSessionSnapshot,
    type IdeScope,
    type IdeStackFrame,
  } from '../ideStore.svelte';
  import { ideDockPrefs } from './ideDockPrefs.svelte';
  import IdeVarRows from './IdeVarRows.svelte';
  import { threadOptionLabel } from './ideDebugPanelModel';
  import { selectDebugThread } from './ideDebugThread';
  import { collectValues, reexpandOpenPaths, VarChangeTracker } from './ideVarTree';

  interface Props {
    dbg: IdeDebugSessionSnapshot;
    showFrames: boolean;
    showVariables: boolean;
    hasTrailingConsole: boolean;
    quickEvalSlot?: Snippet;
  }

  let {
    dbg,
    showFrames,
    showVariables,
    hasTrailingConsole,
    quickEvalSlot,
  }: Props = $props();

  const LS_FRAMES_W = 'otto.ide.dbgFramesW';
  const LS_VARS_W = 'otto.ide.dbgVarsW';
  const FRAMES_DEFAULT_W = 190;

  let framesW = $state(loadPaneWidth(LS_FRAMES_W, FRAMES_DEFAULT_W, 160, 560));
  let varsW = $state(loadPaneWidth(LS_VARS_W, 340, 200, 680));
  /** Open scopes by NAME and open variables by PATH: both outlive a stop. */
  let openScopes = $state<Record<string, boolean>>({});
  let openPaths = $state<Record<string, boolean>>({});
  let loadingRefs = $state<Record<number, boolean>>({});
  let previousValues = $state<Record<string, string>>({});
  let watchDraft = $state('');
  let watchFormOpen = $state(false);
  let watchInputEl = $state<HTMLInputElement>();

  const compact = $derived(ideLayout.stack);
  const isPaused = $derived(dbg.state === 'paused');
  const scopeKey = (s: IdeScope): string => `${s.name}:${s.variables_reference}`;
  const frameName = $derived(dbg.stack_frames.find((f) => f.id === dbg.active_frame_id)?.name ?? '');
  /** Changes on every stop (DAP references are per stop) and on frame selection. */
  const stopKey = $derived(isPaused ? `${dbg.id}:${dbg.stop_seq ?? 0}:${dbg.scopes.map((s) => s.variables_reference).join(',')}` : '');
  const known = (ref: number) => ide.expandedVariables[ref];

  // After each stop: open the first scope the first time, then re-open every
  // open path breadth-first (capped), so `order → items` stays expanded.
  let reexpandedFor = '';
  $effect(() => {
    const key = stopKey;
    if (!key) return;
    const scopes = dbg.scopes;
    untrack(() => {
      if (reexpandedFor === key) return;
      reexpandedFor = key;
      const first = scopes[0];
      if (first && openScopes[first.name] === undefined) openScopes = { ...openScopes, [first.name]: true };
      void reexpandOpenPaths(scopes, openScopes, openPaths, known, async (ref) => {
        loadingRefs = { ...loadingRefs, [ref]: true };
        try { return await ide.expandVariables(ref); }
        finally { loadingRefs = { ...loadingRefs, [ref]: false }; }
      }).catch(() => {});
    });
  });

  // Values by `frame|path`, rotated into "previous" when the stop changes.
  const tracker = new VarChangeTracker();
  const visibleValues = $derived(collectValues(dbg.scopes, openScopes, openPaths, known));
  $effect(() => {
    const key = stopKey;
    const vals = visibleValues;
    const fn = frameName;
    if (!key) return;
    untrack(() => {
      const prefixed: Record<string, string> = {};
      for (const [p, v] of Object.entries(vals)) prefixed[`${fn}|${p}`] = v;
      tracker.observe(key, prefixed);
      previousValues = tracker.previous;
    });
  });

  function isChanged(path: string, value: string): boolean {
    const k = `${frameName}|${path}`;
    return Object.prototype.hasOwnProperty.call(previousValues, k) && previousValues[k] !== value;
  }

  function setOpen(path: string, open: boolean): void {
    openPaths = { ...openPaths, [path]: open };
  }

  async function toggleScope(s: IdeScope): Promise<void> {
    const next = !openScopes[s.name];
    openScopes = { ...openScopes, [s.name]: next };
    const ref = s.variables_reference;
    if (!next || ref <= 0 || ide.expandedVariables[ref] || loadingRefs[ref]) return;
    loadingRefs = { ...loadingRefs, [ref]: true };
    try { await ide.expandVariables(ref); }
    finally { loadingRefs = { ...loadingRefs, [ref]: false }; }
  }

  function onSelectFrame(frame: IdeStackFrame): void {
    void ide.selectFrame(frame.id, dbg.active_thread_id ?? undefined);
    if (frame.path) {
      if (ide.activePath) pushIdeNavLocation(ide.activePath, ide.cursorInfo.line, ide.cursorInfo.col);
      void ide.openFile(frame.path, { line: frame.line, col: frame.column ?? 1, preview: false });
    }
  }

  function onThreadChange(e: Event): void {
    const id = Number((e.currentTarget as HTMLSelectElement).value);
    // The thread only: the daemon picks that thread's top frame (a frame id of
    // the current thread does not exist on another one).
    if (Number.isFinite(id)) void selectDebugThread(id);
  }

  function onAddWatch(e: SubmitEvent): void {
    e.preventDefault();
    const expr = watchDraft.trim();
    if (!expr) return;
    ide.addWatch(expr);
    watchDraft = '';
    watchFormOpen = false;
  }

  async function openWatchForm(): Promise<void> {
    watchFormOpen = true;
    await Promise.resolve();
    watchInputEl?.focus();
  }

  function onWatchBlur(): void {
    if (!watchDraft.trim()) watchFormOpen = false;
  }

  const showWatchForm = $derived(watchFormOpen || ide.watches.length === 0);
</script>

{#if showFrames}
  {@const fillFrames = !showVariables && !hasTrailingConsole}
  <section
    class="dbg-col"
    class:is-fill={fillFrames}
    class:is-compact={compact}
    style:inline-size={!fillFrames && !compact ? `${framesW}px` : undefined}
    aria-label="Threads and call stack"
    data-testid="ide-debug-frames-pane"
  >
    <header class="dbg-col-head">
      <h2 class="dbg-col-title">Frames</h2>
      {#if dbg.threads.length > 1}
        <select class="input dbg-thread-select" dir="ltr" aria-label="Active thread" value={dbg.active_thread_id ?? dbg.threads[0]?.id} onchange={onThreadChange} data-testid="ide-debug-thread-select">
          {#each dbg.threads as th (th.id)}
            <option value={th.id}>{threadOptionLabel(th, { id: dbg.active_thread_id, reason: isPaused ? dbg.stopped_reason : null })}</option>
          {/each}
        </select>
      {:else if dbg.threads.length === 1}
        <span class="dbg-thread-one mono dim" dir="ltr" title={threadOptionLabel(dbg.threads[0], { id: dbg.active_thread_id, reason: isPaused ? dbg.stopped_reason : null })}>{dbg.threads[0].name}</span>
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
            {#if frame.path}<span class="dbg-frame-loc mono" dir="ltr" title={`${frame.path}:${frame.line}`}>{ideBasename(frame.path)}:{frame.line}</span>{/if}
          </button>
        {/each}
      {/if}
    </div>
  </section>

  {#if (showVariables || hasTrailingConsole) && !compact}
    <PaneDivider bind:width={framesW} storageKey={LS_FRAMES_W} min={160} max={560} defaultWidth={FRAMES_DEFAULT_W} label="Resize Frames column" />
  {/if}
{/if}

{#if showVariables}
  {@const fillVars = !hasTrailingConsole}
  <section
    class="dbg-col"
    class:is-fill={fillVars}
    class:is-compact={compact}
    style:inline-size={!fillVars && !compact ? `${varsW}px` : undefined}
    aria-label="Variables and watches"
    data-testid="ide-debug-variables-pane"
  >
    <header class="dbg-col-head">
      <h2 class="dbg-col-title">Variables{ideDockPrefs.dbgShowWatches ? ' & watches' : ''}</h2>
      <div class="dbg-head-actions">
        <button
          type="button"
          class="dbg-pane-btn"
          class:is-on={ideDockPrefs.dbgShowWatches}
          aria-pressed={ideDockPrefs.dbgShowWatches}
          title={ideDockPrefs.dbgShowWatches ? 'Hide Watches section' : 'Show Watches section'}
          onclick={() => ideDockPrefs.toggleWatches()}
          data-testid="ide-debug-toggle-watches"
        >Watches{ide.watches.length > 0 ? ` (${ide.watches.length})` : ''}</button>
        {#if ideDockPrefs.dbgShowWatches && ide.watches.length > 0}
          <button type="button" class="icon-btn" aria-label="Refresh watches" title="Refresh watches" disabled={!isPaused} onclick={() => void ide.refreshWatches()}>
            <Icon name="refresh" size={12} />
          </button>
        {/if}
      </div>
    </header>
    <div class="dbg-scroll" role="list" aria-label="Variables">
      {#if dbg.scopes.length === 0}
        <p class="dbg-muted">{isPaused ? 'No variable scopes in this frame.' : 'Variables populate when paused at a breakpoint.'}</p>
      {:else}
        {#each dbg.scopes as scope (scopeKey(scope))}
          {@const isOpen = Boolean(openScopes[scope.name])}
          {@const vars = ide.expandedVariables[scope.variables_reference] ?? []}
          <div class="dbg-scope" role="listitem">
            <button type="button" class="dbg-scope-head" aria-expanded={isOpen} onclick={() => void toggleScope(scope)}>
              <Icon name={isOpen ? 'chevronDown' : 'chevronRight'} size={12} />
              <span class="dbg-scope-name">{scope.name}</span>
              {#if scope.expensive}<Badge tone="neutral" variant="outline" label="expensive" />{/if}
            </button>
            {#if isOpen}
              {#if loadingRefs[scope.variables_reference] && vars.length === 0}
                <div class="dbg-var-empty dim"><span class="spinner" style="--spinner-size: 12px" aria-hidden="true"></span><span>Inspecting scope…</span></div>
              {:else if vars.length === 0}
                <div class="dbg-var-empty dim">No variables in {scope.name}</div>
              {:else}
                <div role="list" aria-label={`${scope.name} variables`}>
                  <IdeVarRows vars={vars} root={scope.name} {openPaths} onsetopen={setOpen} changed={isChanged} />
                </div>
              {/if}
            {/if}
          </div>
        {/each}
      {/if}

      {#if ideDockPrefs.dbgShowWatches}
        <div class="dbg-watches" role="listitem">
          <div class="dbg-subhead">
            <span>Watches</span>
            {#if !showWatchForm}
              <button type="button" class="icon-btn" aria-label="Add watch" title="Add watch" onclick={() => void openWatchForm()} data-testid="ide-debug-add-watch">
                <Icon name="plus" size={12} />
              </button>
            {/if}
          </div>
          {#if showWatchForm}
            <form class="dbg-inline-form" onsubmit={onAddWatch}>
              <input bind:this={watchInputEl} class="input dbg-inline-input" type="text" dir="ltr" aria-label="Add watch expression" placeholder="Add watch expression…" bind:value={watchDraft} onblur={onWatchBlur} />
              <button type="submit" class="btn small" disabled={!watchDraft.trim()}><Icon name="plus" size={12} /><span>Watch</span></button>
            </form>
          {/if}
          {#each ide.watches as w (w.id)}
            <div class="dbg-watch-row reveal-host">
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
      {/if}
    </div>
    {#if quickEvalSlot}{@render quickEvalSlot()}{/if}
  </section>

  {#if hasTrailingConsole && !compact}
    <PaneDivider bind:width={varsW} storageKey={LS_VARS_W} min={200} max={680} defaultWidth={340} label="Resize Variables column" />
  {/if}
{/if}

<style>
  .dbg-col { display: flex; flex-direction: column; flex-shrink: 0; min-block-size: 0; min-inline-size: 160px; border-inline-end: 1px solid var(--border); }
  .dbg-col.is-fill { flex: 1; flex-shrink: 1; border-inline-end: 0; }
  .dbg-col.is-compact { inline-size: 100%; border-inline-end: 0; border-block-end: 1px solid var(--border); min-block-size: 150px; }
  .dbg-col-head { display: flex; align-items: center; justify-content: space-between; gap: 8px; block-size: 28px; min-block-size: 28px; padding-inline: 10px; border-block-end: 1px solid var(--border); background: var(--surface-2); flex-shrink: 0; }
  .dbg-col-title, .dbg-subhead { margin: 0; font-size: var(--fs-xs); font-weight: 600; color: var(--text-dim); text-transform: uppercase; letter-spacing: 0.06em; }
  .dbg-subhead { display: flex; align-items: center; justify-content: space-between; gap: 6px; padding: 4px 10px; }
  .dbg-head-actions { display: flex; align-items: center; gap: 4px; }
  .dbg-pane-btn {
    display: inline-flex; align-items: center; gap: 4px; block-size: 22px; padding-inline: 6px;
    border: 1px solid var(--border); border-radius: var(--radius-s);
    background: var(--surface-2); color: var(--text-dim); font-size: var(--fs-xs); white-space: nowrap; cursor: pointer;
  }
  .dbg-pane-btn:hover { background: var(--hover); color: var(--text); }
  .dbg-pane-btn.is-on { border-color: var(--accent-line); background: var(--accent-soft); color: var(--accent-text); }
  .dbg-thread-select { block-size: 22px; font-size: var(--fs-xs); max-inline-size: 160px; min-inline-size: 0; }
  .dbg-thread-one { font-size: var(--fs-xs); overflow: hidden; text-overflow: ellipsis; white-space: nowrap; min-inline-size: 0; }
  .dbg-scroll { flex: 1; min-block-size: 0; overflow-y: auto; padding-block: 4px; }
  .dbg-muted { margin: 0; padding: 12px; font-size: var(--fs-xs); color: var(--text-dim); }
  .dbg-frame { display: flex; align-items: center; justify-content: space-between; gap: 8px; inline-size: 100%; padding: 4px 10px; border: 0; background: transparent; color: var(--text); text-align: start; cursor: pointer; font-size: var(--fs-xs); }
  .dbg-frame:hover { background: var(--hover); }
  .dbg-frame.is-selected { background: var(--accent-soft); color: var(--accent-text); }
  .dbg-frame-fn { flex: 1; font-size: var(--fs-xs); }
  .dbg-frame-loc { flex-shrink: 0; font-size: var(--fs-xs); color: var(--text-dim); font-variant-numeric: tabular-nums; }
  .dbg-scope { border-block-end: 1px solid var(--separator); }
  .dbg-scope-head { display: flex; align-items: center; gap: 6px; inline-size: 100%; padding: 4px 8px; border: 0; background: transparent; color: var(--text); font-size: var(--fs-s); font-weight: 500; text-align: start; cursor: pointer; }
  .dbg-scope-head:hover { background: var(--hover); }
  .dbg-scope-name { flex: 1; }
  .dbg-watch-row { display: flex; align-items: center; gap: 4px; min-block-size: 24px; padding-block: 2px; padding-inline: 12px 6px; font-size: var(--fs-xs); }
  .dbg-watch-row:hover { background: var(--hover); }
  .dbg-var-name { color: var(--accent-text); font-size: var(--fs-xs); flex-shrink: 0; }
  .dbg-var-eq { font-size: var(--fs-xs); flex-shrink: 0; }
  .dbg-var-val { flex: 1; min-inline-size: 0; color: var(--text); font-size: var(--fs-xs); }
  .dbg-var-type { color: var(--text-dim); font-size: var(--fs-xs); max-inline-size: 110px; }
  .dbg-var-empty { display: flex; align-items: center; gap: 6px; padding: 4px 24px; font-size: var(--fs-xs); }
  .dbg-watches { padding-block-start: 6px; }
  .dbg-inline-form { display: flex; align-items: center; gap: 6px; padding: 4px 8px; }
  .dbg-inline-input { flex: 1; min-inline-size: 0; block-size: 24px; font-size: var(--fs-xs); }
  .dbg-watch-err { flex: 1; color: var(--danger); font-size: var(--fs-xs); }
  .dbg-col.is-compact .dbg-frame { min-block-size: var(--hit-min); }
</style>
