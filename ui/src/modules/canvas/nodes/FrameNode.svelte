<script lang="ts">
  // Frame node — a labelled bounding box that groups other nodes and defines a
  // slide viewport for Present mode. Rendered behind everything (z 0 via
  // sceneToFlow) as a dashed container with a title chip. Also used for `group`
  // nodes. Not connectable — frames don't carry edges — so no Handles.
  // Double-click the title chip to rename the frame.
  import type { CanvasNode } from '../types';
  import { canvas } from '../../../lib/stores/canvas.svelte';

  interface Props {
    id: string;
    data: { node: CanvasNode };
    selected?: boolean;
  }
  let { id, data, selected }: Props = $props();

  const node = $derived(data.node);
  const label = $derived(node.label ?? 'Frame');

  let editing = $state(false);
  let draft = $state('');
  function startEdit(e: MouseEvent): void {
    e.stopPropagation();
    draft = label;
    editing = true;
  }
  function commit(): void {
    editing = false;
    if (!canvas.scene) return;
    const patched: CanvasNode = { ...node, label: draft };
    canvas.setScene({
      ...canvas.scene,
      nodes: canvas.scene.nodes.map((n) => (n.id === id ? patched : n)),
    });
  }
</script>

<div class="frame" class:selected>
  <!-- svelte-ignore a11y_no_static_element_interactions -->
  <div class="chip" ondblclick={startEdit}>
    {#if editing}
      <!-- svelte-ignore a11y_autofocus -->
      <input
        bind:value={draft}
        autofocus
        onblur={commit}
        onkeydown={(e) => {
          if (e.key === 'Enter') commit();
          else if (e.key === 'Escape') editing = false;
        }}
      />
    {:else}
      <span>{label}</span>
    {/if}
  </div>
</div>

<style>
  .frame {
    width: 100%;
    height: 100%;
    border: 1.5px dashed var(--border);
    border-radius: var(--radius-m);
    /* Faint tint so the frame area is visible but never hides nodes above it. */
    background: color-mix(in srgb, var(--surface-2) 40%, transparent);
    position: relative;
  }
  .frame.selected {
    border-color: var(--accent);
  }
  .chip {
    position: absolute;
    top: -11px;
    left: 8px;
    background: var(--surface);
    border: 1px solid var(--border);
    border-radius: var(--radius-s);
    padding: 1px 8px;
    font-size: 11px;
    color: var(--text-dim);
    line-height: 1.6;
    max-width: 80%;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .chip input {
    border: none;
    outline: none;
    background: transparent;
    color: var(--text);
    font: inherit;
    width: 120px;
  }
</style>
