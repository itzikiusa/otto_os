<script lang="ts">
  // The desktop Context sidebar for a run with a context dir (R1 + Agents):
  // Files (the deliverable and the run's context-file tree) and Agents (the
  // run's live agent sessions, embedded). Resizable from its left edge.
  import Icon from '../../lib/components/Icon.svelte';
  import { startMouseDrag } from '../../lib/dragCursor';
  import { paneResizer } from '../../lib/paneResizer';
  import { ui, WF_CTX_MIN, WF_CTX_MAX } from '../../lib/stores/ui.svelte';
  import Tabs from '../../lib/components/Tabs.svelte';
  import FileTree from '../panels/FileTree.svelte';
  import RunAgents from './RunAgents.svelte';
  import RunFinalOutput from './RunFinalOutput.svelte';
  import { contextFs, type RunSync } from './runSync.svelte';
  import type { WorkflowRun } from '../../lib/api/types';

  interface Props {
    mon: RunSync;
    /** The run on screen, and its context dir. */
    run: WorkflowRun;
    contextDir: string;
    nodeName: (id: string) => string;
    /** The session a step's "Open" asked the Agents tab to focus. */
    focusSid: string | null;
  }
  let { mon, run, contextDir, nodeName, focusSid }: Props = $props();

  // Total agent sessions the current run spawned — the Agents-tab badge.
  const runSessionCount = $derived(
    run.nodes.reduce((a, n) => a + (n.sessions?.length ?? 0), 0),
  );
  const pxText = (v: number): string => `${Math.round(v)} pixels wide`;
  // Context-files sidebar width: drag its left edge (anchored right → dragging
  // left widens it). (R1)
  function startCtxResize(e: MouseEvent): void {
    e.preventDefault();
    const startX = e.clientX;
    const startW = ui.wfCtxWidth;
    startMouseDrag(e, { onMove: (ev) => ui.setWfCtxWidth(startW + (startX - ev.clientX)) });
  }
</script>

<!-- Right sidebar (R1 + Agents): desktop-only, for a run with a context dir.
     Tabbed — Files (the run's context-file tree) and Agents (the run's live
     agent sessions, embedded). Collapse via the header "Panel" button so there's
     no second full-height rail beside the app shell's right rail. Resizable. -->
<aside class="ctx-sidebar" style="width:{ui.wfCtxWidth}px" data-testid="ctx-sidebar">
  <div
    class="ctx-resize"
    role="separator"
    aria-label="Resize context panel"
    title="Drag or use ←/→ to resize"
    use:paneResizer={{ value: ui.wfCtxWidth, min: WF_CTX_MIN, max: WF_CTX_MAX, invert: true, onChange: (w) => ui.setWfCtxWidth(w), onDragStart: startCtxResize, text: pxText }}
  ></div>
  <div class="ctx-head">
    <!-- The shared tab strip (components.md §3): roving focus, ←/→, Home/End. -->
    <Tabs
      label="Context panel sections"
      size="s"
      value={ui.wfCtxTab}
      onchange={(id) => ui.setWfCtxTab(id)}
      tabs={[
        { id: 'files', label: 'Files', icon: 'folder' },
        { id: 'agents', label: 'Agents', icon: 'terminal', count: runSessionCount > 0 ? runSessionCount : undefined, testid: 'ctx-tab-agents' },
      ]}
    />
    <span class="grow"></span>
    <button
      class="icon-btn"
      data-testid="ctx-collapse"
      onclick={() => ui.toggleWfCtx()}
      title="Collapse panel"
      aria-label="Collapse panel"
    >
      <Icon name="panel" size={13} />
    </button>
  </div>
  {#if ui.wfCtxTab === 'files'}
    <div class="ctx-pathline"><code class="ctx-path dim" title={contextDir}>{contextDir}</code></div>
    <RunFinalOutput {mon} {run} />
    <div class="ctx-body">
      {#key contextDir}
        <FileTree root={contextDir} primary={false} {...contextFs(run.id, contextDir)} />
      {/key}
    </div>
  {:else}
    <div class="ctx-body">
      <RunAgents {run} nodeName={(id) => nodeName(id)} {focusSid} />
    </div>
  {/if}
</aside>

<style>
  .grow {
    flex: 1;
  }

  /* Context-files sidebar (R1): a full-height file viewer on the right edge. */
  .ctx-sidebar {
    position: relative;
    flex-shrink: 0;
    height: 100%;
    display: flex;
    flex-direction: column;
    border-inline-start: 1px solid var(--border);
    background: var(--bg);
    min-height: 0;
  }
  .ctx-resize {
    position: absolute;
    inset-inline-start: -3px;
    top: 0;
    bottom: 0;
    width: 7px;
    cursor: col-resize;
    z-index: 5;
  }
  .ctx-resize:hover,
  .ctx-resize:focus-visible {
    background: linear-gradient(
      to right,
      transparent,
      var(--accent-line),
      transparent
    );
  }
  .ctx-head {
    display: flex;
    align-items: center;
    gap: 6px;
    padding-block: 6px 6px; padding-inline: 10px 8px;
    border-bottom: 1px solid var(--border);
    background: var(--surface);
    flex-shrink: 0;
    min-width: 0;
  }
  .ctx-pathline {
    padding: 4px 10px;
    border-bottom: 1px solid var(--border);
    flex-shrink: 0;
  }
  .ctx-path {
    display: block;
    min-width: 0;
    font-size: var(--fs-xs);
    font-family: var(--font-mono);
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    direction: ltr;
  }
  .ctx-body {
    flex: 1;
    min-height: 0;
    overflow: hidden;
    display: flex;
    flex-direction: column;
  }
  .ctx-resize:focus-visible {
    outline: none;
  }
</style>
