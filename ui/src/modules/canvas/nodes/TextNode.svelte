<script lang="ts">
  // Text node — a free-floating text label. Double-click to edit; align/size are
  // driven by the Inspector. Connectable so text can anchor an arrow.
  import { Handle, Position } from '@xyflow/svelte';
  import type { CanvasNode } from '../types';
  import { canvas } from '../../../lib/stores/canvas.svelte';

  interface Props {
    id: string;
    data: { node: CanvasNode };
    selected?: boolean;
  }
  let { id, data, selected }: Props = $props();

  const node = $derived(data.node);
  const value = $derived(node.text?.value ?? '');
  const align = $derived(node.text?.align ?? 'left');
  const size = $derived(node.text?.size ?? 16);

  let editing = $state(false);
  let draft = $state('');

  function startEdit(): void {
    draft = value;
    editing = true;
  }
  function commit(): void {
    editing = false;
    if (!canvas.scene) return;
    const patched: CanvasNode = { ...node, text: { ...node.text, value: draft } };
    canvas.setScene({
      ...canvas.scene,
      nodes: canvas.scene.nodes.map((n) => (n.id === id ? patched : n)),
    });
  }
</script>

<div
  class="textnode"
  class:selected
  role="button"
  tabindex="-1"
  aria-label={value || 'Text'}
  style:text-align={align}
  style:font-size={`${size}px`}
  ondblclick={startEdit}
>
  <Handle type="target" position={Position.Left} />
  {#if editing}
    <!-- svelte-ignore a11y_autofocus -->
    <textarea
      bind:value={draft}
      autofocus
      style:text-align={align}
      style:font-size={`${size}px`}
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
    <div class="body">{value || 'Text'}</div>
  {/if}
  <Handle type="source" position={Position.Right} />
</div>

<style>
  .textnode {
    width: 100%;
    height: 100%;
    display: flex;
    align-items: center;
    color: var(--text);
    padding: 4px 6px;
    border-radius: var(--radius-s);
  }
  .textnode.selected {
    outline: 1px solid var(--accent);
  }
  .body {
    width: 100%;
    line-height: 1.35;
    white-space: pre-wrap;
    word-break: break-word;
  }
  textarea {
    width: 100%;
    height: 100%;
    resize: none;
    border: none;
    outline: 1px solid var(--accent);
    border-radius: var(--radius-s);
    background: var(--surface);
    color: var(--text);
    font-family: inherit;
    line-height: 1.35;
    padding: 2px;
  }
</style>
