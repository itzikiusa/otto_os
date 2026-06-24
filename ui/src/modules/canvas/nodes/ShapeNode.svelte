<script lang="ts">
  // Shape node — renders the 7 shape variants as an SVG that fills the node box,
  // with an editable centered label. Connectable (left target / right source).
  // Double-click the label to edit it; the edit commits the patched node back to
  // the store via `setScene` (the editor reloads on the resulting `rev` bump).
  import { Handle, Position } from '@xyflow/svelte';
  import type { CanvasNode, ShapeVariant } from '../types';
  import { canvas } from '../../../lib/stores/canvas.svelte';
  import Resizer from './Resizer.svelte';

  interface Props {
    id: string;
    data: { node: CanvasNode };
    selected?: boolean;
    width?: number;
    height?: number;
  }
  let { id, data, selected, width, height }: Props = $props();

  const node = $derived(data.node);
  const variant = $derived<ShapeVariant>(node.shape?.variant ?? 'rect');
  const fill = $derived(node.shape?.fill ?? 'var(--surface)');
  const stroke = $derived(node.shape?.stroke ?? 'var(--border-strong, var(--text-dim))');
  // Box dimensions (xyflow passes width/height; fall back to the model).
  const w = $derived(width ?? node.w ?? 160);
  const h = $derived(height ?? node.h ?? 90);

  let editing = $state(false);
  let draft = $state('');

  function startEdit(): void {
    draft = node.label ?? '';
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

  // SVG path/element for the variant, sized to the box with a small inset so the
  // 1.5px stroke isn't clipped.
  const PAD = 2;
  const innerW = $derived(Math.max(2, w - PAD * 2));
  const innerH = $derived(Math.max(2, h - PAD * 2));
</script>

<div
  class="shape"
  class:selected
  role="button"
  tabindex="-1"
  aria-label={node.label || variant}
  ondblclick={startEdit}
>
  <Resizer {id} visible={selected} minWidth={60} minHeight={40} />
  <Handle type="target" position={Position.Left} />
  <svg width={w} height={h} viewBox={`0 0 ${w} ${h}`} preserveAspectRatio="none">
    {#if variant === 'rect'}
      <rect x={PAD} y={PAD} width={innerW} height={innerH} {fill} {stroke} stroke-width="1.5" />
    {:else if variant === 'roundrect'}
      <rect
        x={PAD}
        y={PAD}
        width={innerW}
        height={innerH}
        rx="12"
        ry="12"
        {fill}
        {stroke}
        stroke-width="1.5"
      />
    {:else if variant === 'ellipse'}
      <ellipse
        cx={w / 2}
        cy={h / 2}
        rx={innerW / 2}
        ry={innerH / 2}
        {fill}
        {stroke}
        stroke-width="1.5"
      />
    {:else if variant === 'diamond'}
      <polygon
        points={`${w / 2},${PAD} ${w - PAD},${h / 2} ${w / 2},${h - PAD} ${PAD},${h / 2}`}
        {fill}
        {stroke}
        stroke-width="1.5"
      />
    {:else if variant === 'triangle'}
      <polygon
        points={`${w / 2},${PAD} ${w - PAD},${h - PAD} ${PAD},${h - PAD}`}
        {fill}
        {stroke}
        stroke-width="1.5"
      />
    {:else if variant === 'cylinder'}
      <!-- DB cylinder: body + top ellipse. -->
      <path
        d={`M${PAD},${10} v${innerH - 20} a${innerW / 2},10 0 0 0 ${innerW},0 v${-(innerH - 20)}`}
        {fill}
        {stroke}
        stroke-width="1.5"
      />
      <ellipse cx={w / 2} cy={10} rx={innerW / 2} ry="10" {fill} {stroke} stroke-width="1.5" />
    {:else if variant === 'parallelogram'}
      <polygon
        points={`${PAD + 24},${PAD} ${w - PAD},${PAD} ${w - PAD - 24},${h - PAD} ${PAD},${h - PAD}`}
        {fill}
        {stroke}
        stroke-width="1.5"
      />
    {/if}
  </svg>

  <div class="label">
    {#if editing}
      <!-- svelte-ignore a11y_autofocus -->
      <textarea
        bind:value={draft}
        autofocus
        onblur={commit}
        onkeydown={(e) => {
          if (e.key === 'Enter' && !e.shiftKey) {
            e.preventDefault();
            commit();
          } else if (e.key === 'Escape') {
            editing = false;
          }
        }}
      ></textarea>
    {:else}
      <span>{node.label}</span>
    {/if}
  </div>
  <Handle type="source" position={Position.Right} />
</div>

<style>
  .shape {
    position: relative;
    width: 100%;
    height: 100%;
  }
  .shape svg {
    display: block;
    width: 100%;
    height: 100%;
  }
  .shape.selected svg {
    filter: drop-shadow(0 0 0 var(--accent));
  }
  .label {
    position: absolute;
    inset: 6px;
    display: grid;
    place-items: center;
    pointer-events: none;
    font-size: 13px;
    line-height: 1.3;
    color: var(--text);
    text-align: center;
    overflow: hidden;
  }
  .label span {
    max-width: 100%;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: pre-wrap;
    word-break: break-word;
  }
  .label textarea {
    pointer-events: auto;
    width: 92%;
    height: 80%;
    resize: none;
    border: none;
    outline: 1px solid var(--accent);
    border-radius: var(--radius-s);
    background: var(--surface);
    color: var(--text);
    font: inherit;
    text-align: center;
    padding: 4px;
  }
</style>
