<script lang="ts">
  // Frames & Threads + Variables & Watches inspector columns for `IdeDebuggerPanel`.
  import type { Snippet } from 'svelte';
  import Badge from '../../../lib/components/Badge.svelte';
  import Icon from '../../../lib/components/Icon.svelte';
  import PaneDivider from '../../../lib/components/PaneDivider.svelte';
  import { copyWithToast } from '../../../lib/clipboard';
  import { loadPaneWidth } from '../../../lib/paneResizer';
  import { viewport } from '../../../lib/stores/viewport.svelte';
  import { pushIdeNavLocation } from '../editor/ideEditorStoreBridge.svelte';
  import {
    ide,
    ideBasename,
    type IdeDebugSessionSnapshot,
    type IdeScope,
    type IdeStackFrame,
    type IdeVariable,
  } from '../ideStore.svelte';
  import { ideDockPrefs } from './ideDockPrefs.svelte';

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

  const VAR_CAP = 100;
  const LS_FRAMES_W = 'otto.ide.dbgFramesW';
  const LS_VARS_W = 'otto.ide.dbgVarsW';

  let framesW = $state(loadPaneWidth(LS_FRAMES_W, 250, 160, 560));
  let varsW = $state(loadPaneWidth(LS_VARS_W, 340, 200, 680));
  let openScopes = $state<Record<string, boolean>>({});
  let openRefs = $state<Record<number, boolean>>({});
  let showAllVars = $state<Record<string, boolean>>({});
  let loadingRefs = $state<Record<number, boolean>>({});
  let watchDraft = $state('');

  const isPaused = $derived(dbg.state === 'paused');
  const scopeKey = (s: IdeScope): string => `${s.name}:${s.variables_reference}`;

  const autoFetchedScopes = new Set<string>();
  $effect(() => {
    const cur = dbg;
    if (cur.state !== 'paused' || !cur.scopes?.length) return;
    const first = cur.scopes[0];
    if (!first || first.variables_reference <= 0) return;
    const key = `${cur.id}:${cur.active_frame_id ?? 0}:${first.name}:${first.variables_reference}`;
    if (autoFetchedScopes.has(key)) return;
    autoFetchedScopes.add(key);
    openScopes = { ...openScopes, [scopeKey(first)]: true };
    if (!ide.expandedVariables[first.variables_reference]) void loadRef(first.variables_reference);
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
    void ide.selectFrame(frame.id, dbg.active_thread_id ?? undefined);
    if (frame.path) {
      if (ide.activePath) pushIdeNavLocation(ide.activePath, ide.cursorInfo.line, ide.cursorInfo.col);
      void ide.openFile(frame.path, { line: frame.line, col: frame.column ?? 1, preview: false });
    }
  }

  function onThreadChange(e: Event): void {
    const id = Number((e.currentTarget as HTMLSelectElement).value);
    const firstFrame = dbg.stack_frames?.[0]?.id;
    if (Number.isFinite(id) && firstFrame !== undefined) void ide.selectFrame(firstFrame, id);
  }

  function onAddWatch(e: SubmitEvent): void {
    e.preventDefault();
    const expr = watchDraft.trim();
    if (!expr) return;
    ide.addWatch(expr);
    watchDraft = '';
  }
</script>

{#snippet variableRows(vars: IdeVariable[], depth: number, capKey: string)}
  {@const capped = !showAllVars[capKey] && vars.length > VAR_CAP ? vars.slice(0, VAR_CAP) : vars}
  {#each capped as v (`${depth}:${v.name}:${v.variables_reference}`)}
    {@const expandable = v.variables_reference > 0}
    {@const expanded = Boolean(openRefs[v.variables_reference])}
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
      <button type="button" class="icon-btn reveal-on-hover" aria-label={`Copy value of ${v.name}`} title={`Copy value of ${v.name}`} onclick={() => void copyWithToast(v.value, v.name)}>
        <Icon name="copy" size={12} />
      </button>
    </div>
    {#if expandable && expanded}
      {#if loadingRefs[v.variables_reference] && children.length === 0}
        <div class="dbg-var-empty dim" style:padding-inline-start={`${26 + depth * 14}px`}>
          <span class="spinner" style="--spinner-size: 12px" aria-hidden="true"></span><span>Inspecting fields…</span>
        </div>
      {:else if children.length === 0}
        <div class="dbg-var-empty dim" style:padding-inline-start={`${26 + depth * 14}px`}>Empty</div>
      {:else}
        {@render variableRows(children, depth + 1, `ref:${v.variables_reference}`)}
      {/if}
    {/if}
  {/each}
  {#if !showAllVars[capKey] && vars.length > VAR_CAP}
    <div class="dbg-var-empty" style:padding-inline-start={`${12 + depth * 14}px`}>
      <button type="button" class="btn small ghost" onclick={() => (showAllVars = { ...showAllVars, [capKey]: true })}>
        Show all {vars.length} items ({vars.length - VAR_CAP} more)
      </button>
    </div>
  {/if}
{/snippet}

{#if showFrames}
  {@const fillFrames = !showVariables && !hasTrailingConsole}
  <section
    class="dbg-col"
    class:is-fill={fillFrames}
    style:inline-size={!fillFrames && !viewport.isPhone ? `${framesW}px` : undefined}
    aria-label="Threads and call stack"
    data-testid="ide-debug-frames-pane"
  >
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
            {#if frame.path}<span class="dbg-frame-loc mono" dir="ltr" title={`${frame.path}:${frame.line}`}>{ideBasename(frame.path)}:{frame.line}</span>{/if}
          </button>
        {/each}
      {/if}
    </div>
  </section>

  {#if (showVariables || hasTrailingConsole) && !viewport.isPhone}
    <PaneDivider bind:width={framesW} storageKey={LS_FRAMES_W} min={160} max={560} defaultWidth={250} label="Resize Frames column" />
  {/if}
{/if}

{#if showVariables}
  {@const fillVars = !hasTrailingConsole}
  <section
    class="dbg-col"
    class:is-fill={fillVars}
    style:inline-size={!fillVars && !viewport.isPhone ? `${varsW}px` : undefined}
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
    <div class="dbg-scroll">
      {#if dbg.scopes.length === 0}
        <p class="dbg-muted">{isPaused ? 'No variable scopes in this frame.' : 'Variables populate when paused at a breakpoint.'}</p>
      {:else}
        {#each dbg.scopes as scope (scopeKey(scope))}
          {@const sKey = scopeKey(scope)}
          {@const isOpen = Boolean(openScopes[sKey])}
          {@const vars = ide.expandedVariables[scope.variables_reference] ?? []}
          <div class="dbg-scope">
            <button type="button" class="dbg-scope-head" aria-expanded={isOpen} onclick={() => toggleScope(scope)}>
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
                {@render variableRows(vars, 0, `scope:${sKey}`)}
              {/if}
            {/if}
          </div>
        {/each}
      {/if}

      {#if ideDockPrefs.dbgShowWatches}
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
      {/if}
    </div>
    {#if quickEvalSlot}{@render quickEvalSlot()}{/if}
  </section>

  {#if hasTrailingConsole && !viewport.isPhone}
    <PaneDivider bind:width={varsW} storageKey={LS_VARS_W} min={200} max={680} defaultWidth={340} label="Resize Variables column" />
  {/if}
{/if}

<style>
  .dbg-col { display: flex; flex-direction: column; flex-shrink: 0; min-block-size: 0; min-inline-size: 160px; border-inline-end: 1px solid var(--border); }
  .dbg-col.is-fill { flex: 1; flex-shrink: 1; border-inline-end: 0; }
  .dbg-col-head { display: flex; align-items: center; justify-content: space-between; gap: 8px; block-size: 28px; min-block-size: 28px; padding-inline: 10px; border-block-end: 1px solid var(--border); background: var(--surface-2); flex-shrink: 0; }
  .dbg-col-title, .dbg-subhead { margin: 0; font-size: var(--fs-xs); font-weight: 600; color: var(--text-dim); text-transform: uppercase; letter-spacing: 0.06em; }
  .dbg-subhead { padding: 4px 10px; }
  .dbg-head-actions { display: flex; align-items: center; gap: 4px; }
  .dbg-pane-btn {
    display: inline-flex; align-items: center; gap: 4px; block-size: 22px; padding-inline: 6px;
    border: 1px solid var(--border); border-radius: var(--radius-s);
    background: var(--surface-2); color: var(--text-dim); font-size: var(--fs-xs); white-space: nowrap; cursor: pointer;
  }
  .dbg-pane-btn:hover { background: var(--hover); color: var(--text); }
  .dbg-pane-btn.is-on { border-color: var(--accent-line); background: var(--accent-soft); color: var(--accent-text); }
  .dbg-thread-select { block-size: 22px; font-size: var(--fs-xs); max-inline-size: 160px; }
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
  @media (max-width: 640px) {
    .dbg-col { inline-size: 100%; border-inline-end: 0; border-block-end: 1px solid var(--border); min-block-size: 150px; }
  }
</style>
