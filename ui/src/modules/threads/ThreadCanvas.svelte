<script lang="ts">
  import Icon from '../../lib/components/Icon.svelte';
  import ProviderIcon, { hasProviderIcon } from '../../lib/components/ProviderIcon.svelte';
  import { startMouseDrag } from '../../lib/dragCursor';
  import { paneResizer, RESIZE_TITLE } from '../../lib/paneResizer';
  import { ws } from '../../lib/stores/workspace.svelte';
  import ThreadPane from './ThreadPane.svelte';
  import { isThreadSession } from './threadsHelpers';
  import { threads } from './threadsStore.svelte';

  interface Props {
    onstartroom: (sessionId: string) => void;
  }
  let { onstartroom }: Props = $props();

  let splitContainer = $state<HTMLDivElement | null>(null);
  let splitRatio = $state(50);

  const panes = $derived(threads.paneIds);
  const availableToAdd = $derived(
    ws.sessions.filter((s) => !s.archived && isThreadSession(s) && !panes.includes(s.id)),
  );

  const effectiveMode = $derived.by(() => {
    if (threads.layoutMode === 'grid' || panes.length >= 3) return 'grid';
    if (threads.layoutMode === 'split' || panes.length === 2) return 'split';
    return 'single';
  });

  function setRatio(pct: number): void {
    splitRatio = Math.max(25, Math.min(75, Math.round(pct)));
  }

  function startSplitDrag(e: MouseEvent): void {
    if (e.button !== 0 || !splitContainer) return;
    const rect = splitContainer.getBoundingClientRect();
    if (rect.width <= 0) return;
    const rtl = getComputedStyle(splitContainer).direction === 'rtl';
    startMouseDrag(e, {
      cursor: 'col-resize',
      onMove: (ev) => {
        const frac = rtl
          ? ((rect.right - ev.clientX) / rect.width) * 100
          : ((ev.clientX - rect.left) / rect.width) * 100;
        setRatio(frac);
      },
    });
  }
</script>

<div
  class="tc-canvas"
  class:single={effectiveMode === 'single'}
  class:split={effectiveMode === 'split'}
  class:grid={effectiveMode === 'grid'}
  data-layout={effectiveMode}
  data-testid="threads-canvas"
  bind:this={splitContainer}
>
  {#if effectiveMode === 'single' && panes[0]}
    <div class="tc-single-slot">
      <ThreadPane
        paneId={panes[0]}
        focused={true}
        multiPane={false}
        {onstartroom}
      />
    </div>
  {:else if effectiveMode === 'split'}
    <div class="tc-split-slot" style:flex="0 0 calc({splitRatio}% - 5px)">
      {#if panes[0]}
        <ThreadPane
          paneId={panes[0]}
          focused={threads.focusedSessionId === panes[0]}
          multiPane={panes.length > 1}
          {onstartroom}
        />
      {/if}
    </div>

    <div
      class="tc-splitter"
      role="separator"
      aria-orientation="vertical"
      aria-label="Resize split threads"
      title={RESIZE_TITLE}
      data-testid="threads-split-divider"
      use:paneResizer={{
        value: splitRatio,
        min: 25,
        max: 75,
        step: 2,
        bigStep: 10,
        onChange: setRatio,
        onReset: () => setRatio(50),
        onDragStart: startSplitDrag,
        text: (v) => `${Math.round(v)}%`,
      }}
    ></div>

    <div class="tc-split-slot tc-split-end">
      {#if panes[1]}
        <ThreadPane
          paneId={panes[1]}
          focused={threads.focusedSessionId === panes[1]}
          multiPane={true}
          {onstartroom}
        />
      {:else}
        <div class="tc-empty-slot" data-testid="threads-split-picker">
          <div class="tc-slot-card">
            <Icon name="split" size={24} />
            <h3>Pick a second thread for Split</h3>
            <p>Watch two agents work side-by-side or relay findings between them.</p>
            {#if availableToAdd.length > 0}
              <div class="tc-pick-list">
                {#each availableToAdd.slice(0, 5) as cand (cand.id)}
                  <button
                    type="button"
                    class="btn small tc-pick-btn"
                    onclick={() => threads.openBeside(cand.id)}
                  >
                    {#if hasProviderIcon(cand.provider)}
                      <ProviderIcon provider={cand.provider} size={13} />
                    {:else}
                      <Icon name="terminal" size={13} />
                    {/if}
                    <span class="ellipsis" dir="auto">{cand.title}</span>
                  </button>
                {/each}
              </div>
            {/if}
            <button
              type="button"
              class="btn small primary"
              onclick={() => threads.startNewThread({ workspaceId: ws.currentId })}
            >
              <Icon name="plus" size={12} /> Start new thread
            </button>
          </div>
        </div>
      {/if}
    </div>
  {:else}
    {#each panes as id (id)}
      <div class="tc-grid-cell">
        <ThreadPane
          paneId={id}
          focused={threads.focusedSessionId === id}
          multiPane={panes.length > 1}
          {onstartroom}
        />
      </div>
    {/each}
    {#if panes.length < 4}
      <div class="tc-grid-cell tc-empty-slot" data-testid="threads-grid-picker">
        <div class="tc-slot-card">
          <Icon name="grid" size={24} />
          <h3>Add a thread ({panes.length}/4)</h3>
          {#if availableToAdd.length > 0}
            <div class="tc-pick-list">
              {#each availableToAdd.slice(0, 4) as cand (cand.id)}
                <button
                  type="button"
                  class="btn small tc-pick-btn"
                  onclick={() => threads.openBeside(cand.id)}
                >
                  {#if hasProviderIcon(cand.provider)}
                    <ProviderIcon provider={cand.provider} size={13} />
                  {/if}
                  <span class="ellipsis" dir="auto">{cand.title}</span>
                </button>
              {/each}
            </div>
          {/if}
          <button
            type="button"
            class="btn small"
            onclick={() => threads.startNewThread({ workspaceId: ws.currentId })}
          >
            <Icon name="plus" size={12} /> New thread
          </button>
        </div>
      </div>
    {/if}
  {/if}
</div>

<style>
  .tc-canvas {
    flex: 1;
    min-width: 0;
    min-height: 0;
    height: 100%;
    padding: 8px;
    background: var(--bg);
    box-sizing: border-box;
  }
  .tc-canvas.single {
    display: flex;
    padding: 0;
  }
  .tc-single-slot {
    flex: 1;
    min-width: 0;
    min-height: 0;
    display: flex;
  }
  .tc-single-slot :global(.tp-pane) {
    flex: 1;
    border: 0;
    border-radius: 0;
  }
  .tc-canvas.split {
    display: flex;
    align-items: stretch;
    gap: 0;
  }
  .tc-split-slot {
    min-width: 0;
    min-height: 0;
    display: flex;
    flex-direction: column;
  }
  .tc-split-end {
    flex: 1 1 0;
  }
  .tc-splitter {
    flex: none;
    position: relative;
    z-index: var(--z-sticky);
    inline-size: 10px;
    cursor: col-resize;
    touch-action: none;
  }
  .tc-splitter::after {
    content: '';
    position: absolute;
    inset-block: 8px;
    inset-inline: 4px;
    border-radius: 999px;
    background: var(--separator);
  }
  .tc-splitter:hover::after,
  .tc-splitter:focus-visible::after {
    inset-inline: 3px;
    background: var(--accent-line-strong);
  }
  .tc-splitter:focus-visible {
    outline: none;
  }
  .tc-canvas.grid {
    display: grid;
    grid-template-columns: repeat(2, minmax(0, 1fr));
    grid-template-rows: repeat(2, minmax(0, 1fr));
    gap: 8px;
  }
  .tc-grid-cell {
    min-width: 0;
    min-height: 0;
    display: flex;
    flex-direction: column;
  }
  .tc-empty-slot {
    display: flex;
    align-items: center;
    justify-content: center;
    border: 1px dashed var(--border-strong);
    border-radius: var(--radius-m);
    background: var(--surface);
    padding: 16px;
  }
  .tc-slot-card {
    display: flex;
    flex-direction: column;
    align-items: center;
    text-align: center;
    gap: 8px;
    max-width: 280px;
    color: var(--text-dim);
  }
  .tc-slot-card h3 {
    margin: 0;
    font-size: var(--fs-m);
    font-weight: 600;
    color: var(--text);
  }
  .tc-slot-card p {
    margin: 0;
    font-size: var(--fs-xs);
    line-height: 1.5;
  }
  .tc-pick-list {
    display: flex;
    flex-direction: column;
    gap: 4px;
    width: 100%;
  }
  .tc-pick-btn {
    width: 100%;
    justify-content: flex-start;
  }
  @media (max-width: 640px) {
    .tc-canvas.split {
      flex-direction: column;
      gap: 8px;
    }
    .tc-split-slot {
      flex: 1 1 0 !important;
    }
    .tc-splitter {
      display: none;
    }
    .tc-canvas.grid {
      grid-template-columns: minmax(0, 1fr);
      grid-auto-rows: minmax(260px, 1fr);
      overflow-y: auto;
    }
  }
</style>
