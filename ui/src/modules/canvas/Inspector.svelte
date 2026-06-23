<script lang="ts">
  // Right inspector — edits the single selected node's properties. Multi-select
  // or empty selection shows a hint. Every edit re-derives the scene through
  // `canvas.setScene` (records history + autosaves + reloads the editor).
  import { canvas } from '../../lib/stores/canvas.svelte';
  import { SHAPE_VARIANTS } from './tools';
  import type { CanvasNode } from './types';

  interface Props {
    selectedIds: string[];
  }
  let { selectedIds }: Props = $props();

  const node = $derived.by((): CanvasNode | null => {
    if (selectedIds.length !== 1 || !canvas.scene) return null;
    return canvas.scene.nodes.find((n) => n.id === selectedIds[0]) ?? null;
  });

  /** Apply a mutation to the selected node and commit the new scene. Used for
   *  discrete controls (color / select / number) that fire once per change. */
  function patch(mutate: (n: CanvasNode) => CanvasNode): void {
    const cur = node;
    if (!cur || !canvas.scene) return;
    const nodes = canvas.scene.nodes.map((n) => (n.id === cur.id ? mutate({ ...n }) : n));
    canvas.setScene({ ...canvas.scene, nodes });
  }

  // Debounced variant for live text fields: `setScene` bumps `rev` (which reloads
  // the editor) + records an undo snapshot, so committing per keystroke would
  // flicker the canvas and bloat history. Each `oninput` captures the FULL new
  // value, so the last debounced call carries the final text. The focused input
  // lives here (not in the reloaded editor), so it keeps the user's typing.
  let textTimer: ReturnType<typeof setTimeout> | null = null;
  function patchText(mutate: (n: CanvasNode) => CanvasNode): void {
    if (textTimer) clearTimeout(textTimer);
    textTimer = setTimeout(() => patch(mutate), 250);
  }
  $effect(() => () => {
    if (textTimer) clearTimeout(textTimer);
  });
</script>

<div class="inspector">
  {#if !node}
    <div class="empty">
      {#if selectedIds.length > 1}
        {selectedIds.length} blocks selected.
      {:else}
        Select a block to edit its color, font, or type.
      {/if}
    </div>
  {:else}
    <div class="section-title">{node.kind}</div>

    {#if node.kind !== 'freehand' && node.kind !== 'image'}
      <label class="field">
        <span>Label</span>
        <input
          value={node.label ?? ''}
          oninput={(e) => patchText((n) => ({ ...n, label: (e.target as HTMLInputElement).value }))}
        />
      </label>
    {/if}

    {#if node.kind === 'shape' && node.shape}
      <label class="field">
        <span>Variant</span>
        <select
          value={node.shape.variant}
          onchange={(e) =>
            patch((n) => ({
              ...n,
              shape: { ...n.shape!, variant: (e.target as HTMLSelectElement).value as never },
            }))}
        >
          {#each SHAPE_VARIANTS as v (v.variant)}
            <option value={v.variant}>{v.label}</option>
          {/each}
        </select>
      </label>
      <label class="field">
        <span>Fill</span>
        <input
          type="color"
          value={node.shape.fill ?? '#ffffff'}
          oninput={(e) =>
            patch((n) => ({ ...n, shape: { ...n.shape!, fill: (e.target as HTMLInputElement).value } }))}
        />
      </label>
      <label class="field">
        <span>Stroke</span>
        <input
          type="color"
          value={node.shape.stroke ?? '#888888'}
          oninput={(e) =>
            patch((n) => ({ ...n, shape: { ...n.shape!, stroke: (e.target as HTMLInputElement).value } }))}
        />
      </label>
    {/if}

    {#if node.kind === 'text' && node.text}
      <label class="field">
        <span>Size</span>
        <input
          type="number"
          min="8"
          max="72"
          value={node.text.size ?? 16}
          oninput={(e) =>
            patch((n) => ({ ...n, text: { ...n.text!, size: Number((e.target as HTMLInputElement).value) } }))}
        />
      </label>
      <label class="field">
        <span>Align</span>
        <select
          value={node.text.align ?? 'left'}
          onchange={(e) =>
            patch((n) => ({
              ...n,
              text: { ...n.text!, align: (e.target as HTMLSelectElement).value as never },
            }))}
        >
          <option value="left">Left</option>
          <option value="center">Center</option>
          <option value="right">Right</option>
        </select>
      </label>
    {/if}

    {#if node.kind === 'sticky' && node.sticky}
      <label class="field">
        <span>Color</span>
        <input
          type="color"
          value={node.sticky.color ?? '#ffe9a8'}
          oninput={(e) =>
            patch((n) => ({ ...n, sticky: { ...n.sticky!, color: (e.target as HTMLInputElement).value } }))}
        />
      </label>
    {/if}

    {#if node.kind === 'code' && node.code}
      <label class="field">
        <span>Language</span>
        <input
          value={node.code.lang ?? ''}
          placeholder="ts, py, rust…"
          oninput={(e) =>
            patchText((n) => ({ ...n, code: { ...n.code!, lang: (e.target as HTMLInputElement).value } }))}
        />
      </label>
    {/if}

    {#if node.kind === 'mermaid' && node.mermaid}
      <label class="field">
        <span>Type</span>
        <input
          value={node.mermaid.kind ?? ''}
          placeholder="sequence, flowchart…"
          oninput={(e) =>
            patchText((n) => ({ ...n, mermaid: { ...n.mermaid!, kind: (e.target as HTMLInputElement).value } }))}
        />
      </label>
      <div class="hint-sm">Edit the diagram source by double-clicking the node.</div>
    {/if}
  {/if}
</div>

<style>
  .inspector {
    width: 220px;
    flex: 0 0 220px;
    border-left: 1px solid var(--border);
    background: var(--surface);
    padding: 10px;
    overflow-y: auto;
    display: flex;
    flex-direction: column;
    gap: 10px;
  }
  .empty {
    color: var(--text-dim, #888);
    font-size: 12px;
    line-height: 1.5;
    padding: 8px 4px;
  }
  .section-title {
    font-size: 11px;
    text-transform: uppercase;
    letter-spacing: 0.04em;
    color: var(--text-dim, #888);
    font-weight: 600;
  }
  .field {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 8px;
    font-size: 12px;
    color: var(--text);
  }
  .field span {
    color: var(--text-dim, #888);
  }
  .field input,
  .field select {
    flex: 1 1 auto;
    max-width: 130px;
    padding: 4px 6px;
    border: 1px solid var(--border);
    border-radius: var(--radius-s, 6px);
    background: var(--bg);
    color: var(--text);
    font-size: 12px;
  }
  .field input[type='color'] {
    padding: 0;
    height: 24px;
    max-width: 40px;
  }
  .hint-sm {
    font-size: 11px;
    color: var(--text-dim, #888);
  }
</style>
