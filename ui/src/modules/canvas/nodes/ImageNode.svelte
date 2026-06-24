<script lang="ts">
  // Image node — shows an inline data-URL image (or a drop/upload placeholder
  // when empty). Double-click an empty node to pick a file; we read it as a data
  // URL straight into the model (no server attachment in this build). Connectable.
  import { Handle, Position } from '@xyflow/svelte';
  import Icon from '../../../lib/components/Icon.svelte';
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
  const dataUrl = $derived(node.image?.dataUrl ?? '');

  let fileInput = $state<HTMLInputElement | null>(null);

  function pick(): void {
    fileInput?.click();
  }
  function onFile(e: Event): void {
    const f = (e.target as HTMLInputElement).files?.[0];
    if (!f) return;
    const reader = new FileReader();
    reader.onload = () => {
      const url = typeof reader.result === 'string' ? reader.result : '';
      if (!url || !canvas.scene) return;
      const patched: CanvasNode = { ...node, image: { ...node.image, dataUrl: url } };
      canvas.setScene({
        ...canvas.scene,
        nodes: canvas.scene.nodes.map((n) => (n.id === id ? patched : n)),
      });
    };
    reader.readAsDataURL(f);
  }
</script>

<!-- svelte-ignore a11y_no_static_element_interactions -->
<div class="image" class:selected ondblclick={pick}>
  <Resizer {id} visible={selected} minWidth={80} minHeight={60} />
  <Handle type="target" position={Position.Left} />
  {#if dataUrl}
    <img src={dataUrl} alt={node.label || 'image'} />
  {:else}
    <button class="placeholder" onclick={pick}>
      <Icon name="file" size={22} />
      <span>Click to add an image</span>
    </button>
  {/if}
  <input
    bind:this={fileInput}
    type="file"
    accept="image/*"
    onchange={onFile}
    style="display:none"
  />
  <Handle type="source" position={Position.Right} />
</div>

<style>
  .image {
    width: 100%;
    height: 100%;
    border: 1px solid var(--border);
    border-radius: var(--radius-m);
    background: var(--surface-2);
    overflow: hidden;
    display: grid;
    place-items: center;
  }
  .image.selected {
    outline: 1px solid var(--accent);
  }
  img {
    width: 100%;
    height: 100%;
    object-fit: contain;
    display: block;
  }
  .placeholder {
    display: flex;
    flex-direction: column;
    align-items: center;
    gap: 6px;
    width: 100%;
    height: 100%;
    background: none;
    border: none;
    cursor: pointer;
    color: var(--text-dim);
    font-size: 12px;
  }
</style>
