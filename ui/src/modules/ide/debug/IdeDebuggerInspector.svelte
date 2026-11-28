<script lang="ts">
  // Frames & Threads + Variables & Watches inspector columns for `IdeDebuggerPanel`
  // (two list panes in the Debug tool window, on the shared rows).
  // Variables keep their expansion across steps (open state keyed by path and
  // re-applied after every stop) and mark values that changed since the last stop.
  import { untrack, type Snippet } from 'svelte';
  import Icon from '../../../lib/components/Icon.svelte';
  import PaneDivider from '../../../lib/components/PaneDivider.svelte';
  import ListPane from '../../../lib/components/side/ListPane.svelte';
  import PhoneBack from '../../../lib/components/side/PhoneBack.svelte';
  import SideRow from '../../../lib/components/side/SideRow.svelte';
  import SideSection from '../../../lib/components/side/SideSection.svelte';
  import { LIST_PANE, loadPaneW, paneKey, paneStorage, type PaneRange } from '../../../lib/components/side/paneKeys';
  import { copyWithToast } from '../../../lib/clipboard';
  import { ctxMenu, type MenuItem } from '../../../lib/contextmenu.svelte';
  import { rowMenu } from '../../../lib/rowMenu';
  import { viewport } from '../../../lib/stores/viewport.svelte';
  import { pushIdeNavLocation } from '../editor/ideEditorStoreBridge.svelte';
  import {
    ide,
    ideBasename,
    type IdeDebugSessionSnapshot,
    type IdeScope,
    type IdeStackFrame,
  } from '../ideStore.svelte';
  import { ideDockPrefs } from './ideDockPrefs.svelte';
  import IdeVarRows from './IdeVarRows.svelte';
  import { activeThreadTitle, threadOptionLabel } from './ideDebugPanelModel';
  import { selectDebugThread } from './ideDebugThread';
  import { collectValues, reexpandOpenPaths, VarChangeTracker } from './ideVarTree';

  interface Props {
    dbg: IdeDebugSessionSnapshot;
    showFrames: boolean;
    showVariables: boolean;
    hasTrailingConsole: boolean;
    quickEvalSlot?: Snippet;
    /** A phone: a frame's variables are pushed over the Frames list (bound: the panel hides its console with them). */
    phoneOpen?: boolean;
  }

  let {
    dbg,
    showFrames,
    showVariables,
    hasTrailingConsole,
    quickEvalSlot,
    phoneOpen = $bindable(false),
  }: Props = $props();

  // Frames is a list pane (the shared range); Variables keeps its own wider range. Old keys read once.
  const VARS_PANE: PaneRange = { default: 340, min: 200, max: 680 };
  let framesW = $state(loadPaneW(paneStorage(), 'ide', 'dbgFrames', LIST_PANE, { w: 'otto.ide.dbgFramesW' }, 280));
  let varsW = $state(loadPaneW(paneStorage(), 'ide', 'dbgVars', VARS_PANE, { w: 'otto.ide.dbgVarsW' }));
  /** Open scopes by NAME and open variables by PATH: both outlive a stop. */
  let openScopes = $state<Record<string, boolean>>({});
  let openPaths = $state<Record<string, boolean>>({});
  let loadingRefs = $state<Record<number, boolean>>({});
  let previousValues = $state<Record<string, string>>({});
  let watchDraft = $state('');
  let watchFormOpen = $state(false);
  let watchInputEl = $state<HTMLInputElement>();

  const phone = $derived(viewport.isPhone);
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
    phoneOpen = true;
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

  const variablesMenu = (): MenuItem[] => [
    { label: 'Watches', checked: ideDockPrefs.dbgShowWatches, action: () => ideDockPrefs.toggleWatches() },
  ];
  const watchMenu = (w: (typeof ide.watches)[number]): MenuItem[] => [
    { label: 'Copy Value', icon: 'copy', action: () => void copyWithToast(w.error || w.value, w.expression) },
    { label: 'Remove Watch', icon: 'trash', action: () => ide.removeWatch(w.id) },
  ];

  const showWatchForm = $derived(watchFormOpen || ide.watches.length === 0);
</script>

<!-- Two list panes inside the Debug tool window (standalone ListPanes: their
     own headers; the page's list is the explorer): Frames (the processes of a
     js-debug session, the thread picker, the frames) and Variables (scopes as
     tree nodes, the Watches section, quick evaluate at the foot). A phone
     pushes: the frames are the list; a tapped frame shows the variables (and
     the console after them) with "‹ Frames". -->
{#if showFrames}
  {@const fillFrames = !showVariables && !hasTrailingConsole}
  <section
    class="dbg-col"
    class:is-fill={fillFrames || phone}
    style:inline-size={!fillFrames && !phone ? `${framesW}px` : undefined}
    hidden={phone && phoneOpen}
    aria-label="Threads and call stack"
    data-testid="ide-debug-frames-pane"
  >
    <ListPane name="Frames" count={dbg.stack_frames.length}>
      {#snippet filterRow()}
        {#if dbg.threads.length > 1}
          <select class="input dbg-thread-select" dir="ltr" aria-label="Active thread" title={activeThreadTitle(dbg.threads, { id: dbg.active_thread_id, reason: isPaused ? dbg.stopped_reason : null })} value={dbg.active_thread_id ?? dbg.threads[0]?.id} onchange={onThreadChange} data-testid="ide-debug-thread-select">
            {#each dbg.threads as th (th.id)}
              <option value={th.id}>{threadOptionLabel(th, { id: dbg.active_thread_id, reason: isPaused ? dbg.stopped_reason : null })}</option>
            {/each}
          </select>
        {:else if dbg.threads.length === 1}
          <span class="dbg-thread-one mono dim" dir="ltr" title={threadOptionLabel(dbg.threads[0], { id: dbg.active_thread_id, reason: isPaused ? dbg.stopped_reason : null })}>{dbg.threads[0].name}</span>
        {/if}
      {/snippet}
      {#if dbg.targets?.length}
        <SideSection title="Processes" count={dbg.targets.length} testid="ide-debug-targets">
          {#each dbg.targets as t (t.id)}
            <div class="side-row dbg-target" title={t.error ? `${t.name}: not attached: ${t.error}` : t.name}>
              <span class="side-row-main">
                <span class="side-row-text"><span class="side-row-title mono" dir="ltr">{t.name}</span></span>
                <span class="side-row-trailing" class:is-error={!!t.error}>{t.error ? 'not attached' : t.paused ? 'paused' : 'running'}</span>
              </span>
            </div>
          {/each}
        </SideSection>
      {/if}
      {#if dbg.stack_frames.length === 0}
        <p class="dbg-muted">{isPaused ? 'No stack frames reported.' : 'Pause execution or hit a breakpoint to inspect stack frames.'}</p>
      {:else}
        {#each dbg.stack_frames as frame (frame.id)}
          {@const sel = dbg.active_frame_id === frame.id}
          <div class="side-row" class:two-line={!!frame.path} class:selected={sel}>
            <button
              type="button"
              class="side-row-main dbg-frame"
              data-side-row
              aria-current={sel ? 'true' : undefined}
              title={frame.path ? `${frame.name} — ${frame.path}:${frame.line}` : frame.name}
              onclick={() => onSelectFrame(frame)}
            >
              <span class="side-row-text">
                <span class="side-row-title mono" dir="ltr">{frame.name}</span>
                {#if frame.path}<span class="side-row-meta mono" dir="ltr">{ideBasename(frame.path)}:{frame.line}</span>{/if}
              </span>
            </button>
          </div>
        {/each}
      {/if}
    </ListPane>
  </section>

  {#if (showVariables || hasTrailingConsole) && !phone}
    <PaneDivider bind:width={framesW} storageKey={paneKey('ide', 'dbgFrames', 'w')} min={LIST_PANE.min} max={LIST_PANE.max} defaultWidth={280} label="Resize Frames" />
  {/if}
{/if}

{#if showVariables}
  {@const fillVars = !hasTrailingConsole}
  <section
    class="dbg-col"
    class:is-fill={fillVars}
    class:is-stacked={phone}
    style:inline-size={!fillVars && !phone ? `${varsW}px` : undefined}
    hidden={phone && showFrames && !phoneOpen}
    aria-label="Variables and watches"
    data-testid="ide-debug-variables-pane"
  >
    {#if phone && showFrames}<div class="dbg-back"><PhoneBack label="Frames" onclick={() => (phoneOpen = false)} testid="ide-debug-frames-back" /></div>{/if}
    <ListPane name="Variables" menu={variablesMenu}>
      <div role="list" aria-label="Variables">
        {#if dbg.scopes.length === 0}
          <p class="dbg-muted">{isPaused ? 'No variable scopes in this frame.' : 'Variables populate when paused at a breakpoint.'}</p>
        {:else}
          {#each dbg.scopes as scope (scopeKey(scope))}
            {@const isOpen = Boolean(openScopes[scope.name])}
            {@const vars = ide.expandedVariables[scope.variables_reference] ?? []}
            <div role="listitem">
              <SideRow title={scope.name} expanded={isOpen} ontoggle={() => void toggleScope(scope)} onclick={() => void toggleScope(scope)}>
                {#snippet trailing()}{#if scope.expensive}expensive{/if}{/snippet}
              </SideRow>
              {#if isOpen}
                {#if loadingRefs[scope.variables_reference] && vars.length === 0}
                  <div class="dbg-var-empty dim"><span class="spinner" style="--spinner-size: 12px" aria-hidden="true"></span><span>Inspecting scope…</span></div>
                {:else if vars.length === 0}
                  <div class="dbg-var-empty dim">No variables in {scope.name}</div>
                {:else}
                  <div role="list" aria-label={`${scope.name} variables`}>
                    <IdeVarRows vars={vars} root={scope.name} parentRef={scope.variables_reference} {openPaths} onsetopen={setOpen} changed={isChanged} depth={1} />
                  </div>
                {/if}
              {/if}
            </div>
          {/each}
        {/if}

        {#if ideDockPrefs.dbgShowWatches}
          <div role="listitem">
            <SideSection title="Watches" count={ide.watches.length} foldable={false}>
              {#snippet actions()}
                {#if ide.watches.length > 0}
                  <button type="button" class="icon-btn" aria-label="Refresh watches" title="Refresh watches" disabled={!isPaused} onclick={() => void ide.refreshWatches()}><Icon name="refresh" size={12} /></button>
                {/if}
                {#if !showWatchForm}
                  <button type="button" class="icon-btn" aria-label="Add watch" title="Add watch" onclick={() => void openWatchForm()} data-testid="ide-debug-add-watch"><Icon name="plus" size={12} /></button>
                {/if}
              {/snippet}
              {#if showWatchForm}
                <form class="dbg-inline-form" onsubmit={onAddWatch}>
                  <input bind:this={watchInputEl} class="input dbg-inline-input" type="text" dir="ltr" aria-label="Add watch expression" placeholder="Add watch expression…" bind:value={watchDraft} onblur={onWatchBlur} />
                  <button type="submit" class="btn small" disabled={!watchDraft.trim()}><Icon name="plus" size={12} /><span>Watch</span></button>
                </form>
              {/if}
              {#each ide.watches as w (w.id)}
                <div class="side-row">
                  <button
                    type="button"
                    class="side-row-main dbg-watch-main"
                    data-side-row
                    title={w.error || w.value}
                    onclick={(e) => ctxMenu.showAt(e.currentTarget as HTMLElement, watchMenu(w))}
                    oncontextmenu={(e) => { e.preventDefault(); ctxMenu.show(e, watchMenu(w)); }}
                    use:rowMenu
                  >
                    <span class="dbg-var-name mono" dir="ltr">{w.expression}</span>
                    <span class="dim">=</span>
                    {#if w.error}
                      <span class="dbg-watch-err mono ellipsis" dir="ltr">{w.error}</span>
                    {:else}
                      <span class="dbg-var-val mono ellipsis" dir="ltr">{w.value}</span>
                    {/if}
                    {#if w.var_type}<span class="side-row-trailing mono" dir="ltr">{w.var_type}</span>{/if}
                  </button>
                  <button type="button" class="icon-btn side-row-more" tabindex="-1" aria-label={`Watch ${w.expression} actions`} title="More" onclick={(e) => ctxMenu.showAt(e.currentTarget as HTMLElement, watchMenu(w), { align: 'end' })}>
                    <Icon name="more" size={14} />
                  </button>
                </div>
              {/each}
            </SideSection>
          </div>
        {/if}
      </div>
    </ListPane>
    {#if quickEvalSlot}{@render quickEvalSlot()}{/if}
  </section>

  {#if hasTrailingConsole && !phone}
    <PaneDivider bind:width={varsW} storageKey={paneKey('ide', 'dbgVars', 'w')} min={VARS_PANE.min} max={VARS_PANE.max} defaultWidth={VARS_PANE.default} label="Resize Variables" />
  {/if}
{/if}

<style>
  .dbg-col { display: flex; flex-direction: column; flex-shrink: 0; min-block-size: 0; min-inline-size: 160px; }
  .dbg-col.is-fill { flex: 1; flex-shrink: 1; }
  .dbg-col.is-stacked { flex: 1 0 auto; inline-size: 100%; min-block-size: 50%; }
  .dbg-col[hidden] { display: none; }
  .dbg-col :global(.side-list) { flex: 1; block-size: auto; }
  .dbg-back { display: flex; align-items: center; flex-shrink: 0; padding-inline: 4px; border-block-end: 1px solid var(--separator); background: var(--bg-sidebar); }
  .dbg-thread-select { block-size: 24px; inline-size: 100%; font-size: var(--fs-xs); }
  .dbg-thread-one { display: block; font-size: var(--fs-xs); overflow: hidden; text-overflow: ellipsis; white-space: nowrap; padding-inline: 8px; }
  .dbg-muted { margin: 0; padding: 12px 8px; font-size: var(--fs-s); color: var(--text-dim); }
  .dbg-target .side-row-main { cursor: default; }
  .dbg-target .is-error { color: var(--danger); }
  .dbg-var-name { color: var(--accent-text); font-size: var(--fs-s); flex-shrink: 0; }
  .dbg-var-val { flex: 1; min-inline-size: 0; color: var(--text); font-size: var(--fs-s); }
  .dbg-watch-main { gap: 4px; }
  .dbg-var-empty { display: flex; align-items: center; gap: 6px; padding-block: 4px; padding-inline: 32px 8px; font-size: var(--fs-xs); }
  .dbg-inline-form { display: flex; align-items: center; gap: 6px; padding: 4px 8px; }
  .dbg-inline-input { flex: 1; min-inline-size: 0; block-size: 24px; font-size: var(--fs-xs); }
  .dbg-watch-err { flex: 1; color: var(--danger); font-size: var(--fs-s); }
</style>
