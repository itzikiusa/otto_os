<script lang="ts">
  // Mermaid node — lazily renders the `mermaid.src` to an SVG (mermaid is dynamic-
  // imported by ./mermaid). Re-render is debounced on src change so editing the
  // source doesn't thrash. Parse/render errors show inline. Double-click opens an
  // inline source editor (Esc cancels, ⌘↵ commits). Connectable.
  import { Handle, Position } from '@xyflow/svelte';
  import type { CanvasNode } from '../types';
  import { canvas } from '../../../lib/stores/canvas.svelte';
  import { renderMermaid } from '../mermaid';
  import Resizer from './Resizer.svelte';

  interface Props {
    id: string;
    data: { node: CanvasNode };
    selected?: boolean;
  }
  let { id, data, selected }: Props = $props();

  const node = $derived(data.node);
  const src = $derived(node.mermaid?.src ?? '');

  let svg = $state<string | null>(null);
  let error = $state<string | null>(null);
  let rendering = $state(false);
  let host = $state<HTMLDivElement | null>(null);
  // A DOM-safe, unique render id per node (mermaid needs a valid id selector).
  const renderId = $derived(`mmd-${id.replace(/[^a-zA-Z0-9_-]/g, '')}`);
  let timer: ReturnType<typeof setTimeout> | null = null;
  let seq = 0;

  // Debounced (180ms) re-render whenever the source changes.
  $effect(() => {
    const s = src; // track
    if (timer) clearTimeout(timer);
    timer = setTimeout(() => void doRender(s), 180);
    return () => {
      if (timer) clearTimeout(timer);
    };
  });

  async function doRender(s: string): Promise<void> {
    const mine = ++seq;
    rendering = true;
    const res = await renderMermaid(`${renderId}-${mine}`, s);
    if (mine !== seq) return; // a newer render superseded this one
    rendering = false;
    if (res.error) {
      error = res.error;
      // Keep the last good SVG visible while showing the error chip.
    } else {
      error = null;
      svg = res.svg ?? null;
    }
  }

  // Expose the rendered <svg> element on the host for Present mode's sequence
  // stepping (it queries `[data-mermaid-id]` to find this node's SVG).
  $effect(() => {
    if (host) host.setAttribute('data-mermaid-id', id);
  });

  let editing = $state(false);
  let draft = $state('');
  function startEdit(): void {
    draft = src;
    editing = true;
  }
  function commit(): void {
    editing = false;
    if (!canvas.scene) return;
    const patched: CanvasNode = { ...node, mermaid: { ...node.mermaid, src: draft } };
    canvas.setScene({
      ...canvas.scene,
      nodes: canvas.scene.nodes.map((n) => (n.id === id ? patched : n)),
    });
  }
</script>

<!-- svelte-ignore a11y_no_static_element_interactions -->
<div class="mermaid" class:selected ondblclick={startEdit}>
  <Resizer {id} visible={selected} minWidth={240} minHeight={160} />
  <Handle type="target" position={Position.Left} />
  {#if error}
    <div class="err" title={error}>Diagram error: {error}</div>
  {/if}
  {#if editing}
    <!-- svelte-ignore a11y_autofocus -->
    <textarea
      bind:value={draft}
      autofocus
      spellcheck="false"
      onblur={commit}
      onkeydown={(e) => {
        if (e.key === 'Enter' && (e.metaKey || e.ctrlKey)) {
          e.preventDefault();
          commit();
        } else if (e.key === 'Escape') {
          editing = false;
        }
      }}
    ></textarea>
  {:else}
    <div class="render" bind:this={host}>
      {#if svg}
        {@html svg}
      {:else if rendering}
        <div class="dim">Rendering…</div>
      {:else if !error}
        <div class="dim">Empty diagram — double-click to edit</div>
      {/if}
    </div>
  {/if}
  <Handle type="source" position={Position.Right} />
</div>

<style>
  .mermaid {
    width: 100%;
    height: 100%;
    display: flex;
    flex-direction: column;
    background: var(--surface);
    border: 1px solid var(--border);
    border-radius: var(--radius-m);
    overflow: hidden;
  }
  .mermaid.selected {
    outline: 1px solid var(--accent);
  }
  .render {
    flex: 1 1 auto;
    /* Top-aligned + horizontally centred so tall diagrams (sequence/flow) read
       from their first row and scroll down, instead of being vertically centred
       with the top cut off. */
    display: flex;
    align-items: flex-start;
    justify-content: center;
    overflow: auto;
    padding: 12px;
    min-height: 0;
  }
  /* Make the injected SVG fit the node width (no distortion) and read on dark
     schemes. width:100% + height:auto keeps mermaid's own aspect ratio. */
  .render :global(svg) {
    display: block;
    max-width: 100%;
    height: auto;
  }
  .err {
    flex: 0 0 auto;
    padding: 4px 8px;
    font-size: 11px;
    color: var(--status-exited);
    background: var(--surface-2);
    border-bottom: 1px solid var(--border);
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
  }
  .dim {
    color: var(--text-dim);
    font-size: 12px;
  }
  textarea {
    flex: 1 1 auto;
    resize: none;
    border: none;
    outline: none;
    background: var(--term-bg);
    color: var(--text);
    font-family: var(--font-mono);
    font-size: 12px;
    line-height: 1.5;
    padding: 8px;
  }
</style>
