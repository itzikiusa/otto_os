<script lang="ts">
  // Sticky note — a colored card with editable text. Double-click to edit. The
  // sticky color is set via the Inspector. Connectable.
  import { Handle, Position } from '@xyflow/svelte';
  import type { CanvasNode } from '../types';
  import { canvas } from '../../../lib/stores/canvas.svelte';
  import Resizer from './Resizer.svelte';

  interface Props {
    id: string;
    data: { node: CanvasNode };
    selected?: boolean;
  }
  let { id, data, selected }: Props = $props();

  const node = $derived(data.node);
  const value = $derived(node.sticky?.value ?? '');
  const color = $derived(node.sticky?.color ?? '#ffe9a8');

  let editing = $state(false);
  let draft = $state('');

  function startEdit(): void {
    draft = value;
    editing = true;
  }
  function commit(): void {
    editing = false;
    if (!canvas.scene) return;
    const patched: CanvasNode = { ...node, sticky: { ...node.sticky, value: draft, color } };
    canvas.setScene({
      ...canvas.scene,
      nodes: canvas.scene.nodes.map((n) => (n.id === id ? patched : n)),
    });
  }
</script>

<div
  class="sticky"
  class:selected
  role="button"
  tabindex="-1"
  aria-label={value || 'Note'}
  style:background={color}
  ondblclick={startEdit}
>
  <Resizer {id} visible={selected} minWidth={100} minHeight={80} />
  <Handle type="target" position={Position.Left} />
  {#if editing}
    <!-- svelte-ignore a11y_autofocus -->
    <textarea
      bind:value={draft}
      autofocus
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
    <div class="body">{value || 'Note…'}</div>
  {/if}
  <Handle type="source" position={Position.Right} />
</div>

<style>
  .sticky {
    width: 100%;
    height: 100%;
    border-radius: var(--radius-s);
    padding: 10px 12px;
    box-shadow: var(--shadow);
    /* Sticky bodies use a fixed dark ink so colored notes stay readable on both
       schemes regardless of --text. */
    color: #2a2a1a;
    font-size: 13px;
    line-height: 1.4;
    overflow: hidden;
  }
  .sticky.selected {
    outline: 2px solid var(--accent);
  }
  .body {
    width: 100%;
    height: 100%;
    white-space: pre-wrap;
    word-break: break-word;
    overflow: hidden;
  }
  textarea {
    width: 100%;
    height: 100%;
    resize: none;
    border: none;
    outline: none;
    background: transparent;
    color: inherit;
    font: inherit;
  }
</style>
