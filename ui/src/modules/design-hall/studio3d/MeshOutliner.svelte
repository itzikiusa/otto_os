<script lang="ts">
  // Sub-mesh hierarchy outliner + real-time game-dev polygon budget meter.
  // Works for both standalone GLB/glTF/OBJ models and scene3d scenes.
  import Icon from '../../../lib/components/Icon.svelte';
  import { plural } from '../../../lib/plural';
  import {
    POLY_BUDGET_PRESETS,
    formatCount,
    polyBudgetStatus,
    type MeshTopologyStats,
    type SubMeshInfo,
  } from '../../../lib/design/scene3d';

  interface Props {
    meshes: SubMeshInfo[];
    stats: MeshTopologyStats;
    activeMeshUid: string | null;
    canComment: boolean;
    onfocus: (uid: string) => void;
    ontogglevis: (uid: string) => void;
    oncommentmesh: (mesh: SubMeshInfo) => void;
  }
  let { meshes, stats, activeMeshUid, canComment, onfocus, ontogglevis, oncommentmesh }: Props = $props();

  let budgetId = $state<string>('stylized_character');
  let filter = $state('');

  const budgetMax = $derived(
    POLY_BUDGET_PRESETS.find((p) => p.id === budgetId)?.maxTris ?? 25_000,
  );
  const budget = $derived(polyBudgetStatus(stats.triangles, budgetMax));
  const filtered = $derived.by(() => {
    const q = filter.trim().toLowerCase();
    return q ? meshes.filter((m) => m.name.toLowerCase().includes(q) || m.materialName.toLowerCase().includes(q)) : meshes;
  });
  /** A name's end stays readable when the row truncates ("Lant… base" / "Lant… body", not two "Lantern b…"). */
  function tailOf(name: string): string {
    const sp = name.lastIndexOf(' ');
    return sp > 0 && name.length - sp <= 12 ? name.slice(sp) : name.slice(-Math.min(6, Math.floor(name.length / 2)));
  }
</script>

<section class="mo" aria-label="Mesh outliner and topology" data-testid="s3d-mesh-outliner">
  <div class="mo-head">
    <span class="k">Topology</span>
    <span class="mo-tri" title="Total triangles">{formatCount(stats.triangles)} tris</span>
  </div>

  <div class="mo-stats">
    <span>{formatCount(stats.vertices)} verts</span>
    <span>{plural(stats.subMeshes, 'mesh', 'meshes')}</span>
    <span>{plural(stats.materials, 'mat')}</span>
    {#if stats.bounds}
      <span title="Bounding box W × H × D (m)">
        {stats.bounds[0]}×{stats.bounds[1]}×{stats.bounds[2]} m
      </span>
    {/if}
  </div>

  <div class="mo-budget">
    <div class="mo-brow">
      <label class="k" for="s3d-poly-budget">Poly budget</label>
      <select id="s3d-poly-budget" class="mo-select" bind:value={budgetId}>
        {#each POLY_BUDGET_PRESETS as p (p.id)}
          <option value={p.id}>{p.label} ({formatCount(p.maxTris)})</option>
        {/each}
      </select>
    </div>
    <div
      class="mo-bar {budget.tone}"
      role="meter"
      aria-valuemin={0}
      aria-valuemax={100}
      aria-valuenow={Math.min(100, budget.pct)}
      aria-label="Polygon budget usage"
      title={budget.label}
    >
      <span class="mo-fill" style:inline-size="{Math.min(100, budget.pct)}%"></span>
    </div>
    <span class="mo-blabel {budget.tone}">{budget.label}</span>
  </div>

  {#if meshes.length > 0}
    <div class="mo-list-head">
      <span class="k">Sub-meshes ({meshes.length})</span>
      {#if meshes.length > 4}
        <input
          dir="auto"
          type="search"
          class="mo-filter"
          placeholder="Filter meshes…"
          aria-label="Filter sub-meshes"
          bind:value={filter}
        />
      {/if}
    </div>
    <ul class="mo-list">
      {#each filtered as m (m.uid)}
        {@const tail = tailOf(m.name)}
        <li class="mo-row" class:sel={activeMeshUid === m.uid} class:off={!m.visible}>
          <button
            type="button"
            class="mo-name"
            onclick={() => onfocus(m.uid)}
            title={`Focus ${m.name} (${m.triangles.toLocaleString()} tris · ${m.materialName})`}
          >
            <Icon name="layers" size={12} />
            <span class="mo-n"><span class="mo-h">{m.name.slice(0, m.name.length - tail.length)}</span><span class="mo-t">{tail}</span></span>
            <span class="mo-c">{formatCount(m.triangles)}</span>
          </button>
          {#if canComment}
            <button
              type="button"
              class="icon-btn"
              aria-label={`Comment on sub-mesh ${m.name}`}
              title={`Comment on ${m.name}`}
              onclick={() => oncommentmesh(m)}
            >
              <Icon name="comment" size={12} />
            </button>
          {/if}
          <button
            type="button"
            class="icon-btn"
            aria-label={m.visible ? `Hide ${m.name}` : `Show ${m.name}`}
            title={m.visible ? 'Hide mesh' : 'Show mesh'}
            aria-pressed={!m.visible}
            onclick={() => ontogglevis(m.uid)}
          >
            <Icon name={m.visible ? 'eye' : 'eyeOff'} size={12} />
          </button>
        </li>
      {/each}
    </ul>
  {/if}
</section>

<style>
  .mo {
    padding: 10px 12px;
    border-block-start: 1px solid var(--border);
    display: flex;
    flex-direction: column;
    gap: 8px;
  }
  .mo-head,
  .mo-brow,
  .mo-list-head {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 6px;
  }
  .k {
    font-size: var(--fs-xs);
    font-weight: 600;
    letter-spacing: 0.06em;
    text-transform: uppercase;
    color: var(--text-dim);
  }
  .mo-brow .k {
    white-space: nowrap;
  }
  .mo-tri {
    font-size: var(--fs-xs);
    font-family: var(--font-mono);
    font-variant-numeric: tabular-nums;
    color: var(--text);
  }
  .mo-stats {
    display: flex;
    flex-wrap: wrap;
    gap: 6px;
    font-size: var(--fs-xs);
    color: var(--text-dim);
    font-variant-numeric: tabular-nums;
  }
  .mo-budget {
    display: flex;
    flex-direction: column;
    gap: 4px;
  }
  .mo-select {
    font: inherit;
    font-size: var(--fs-xs);
    color: var(--text);
    background: var(--bg);
    border: 1px solid var(--border);
    border-radius: var(--radius-s);
    padding: 2px 6px;
    min-inline-size: 0;
    max-inline-size: 148px;
    text-overflow: ellipsis;
  }
  .mo-bar {
    block-size: 6px;
    border-radius: 999px;
    background: var(--surface-2);
    overflow: hidden;
  }
  .mo-fill {
    display: block;
    block-size: 100%;
    background: var(--success);
  }
  .mo-bar.warn .mo-fill {
    background: var(--warning);
  }
  .mo-bar.over .mo-fill {
    background: var(--danger);
  }
  .mo-blabel {
    font-size: var(--fs-xs);
    color: var(--text-dim);
    font-variant-numeric: tabular-nums;
  }
  .mo-blabel.warn {
    color: var(--warning);
  }
  .mo-blabel.over {
    color: var(--danger);
  }
  .mo-filter {
    inline-size: 110px;
    font: inherit;
    font-size: var(--fs-xs);
    padding: 2px 6px;
    border: 1px solid var(--border);
    border-radius: var(--radius-s);
    background: var(--bg);
    color: var(--text);
  }
  .mo-list {
    list-style: none;
    margin: 0;
    padding: 0;
    display: flex;
    flex-direction: column;
    gap: 2px;
    max-block-size: 200px;
    overflow-x: hidden;
    overflow-y: auto;
  }
  .mo-row {
    min-inline-size: 0;
    display: flex;
    align-items: center;
    gap: 2px;
    border-radius: var(--radius-s);
  }
  .mo-row.sel {
    background: var(--accent-soft);
  }
  .mo-row.off {
    opacity: var(--disabled-opacity);
  }
  .mo-name {
    flex: 1;
    min-inline-size: 0;
    display: flex;
    align-items: center;
    gap: 6px;
    padding: 4px 6px;
    border: 0;
    border-radius: var(--radius-s);
    background: transparent;
    color: var(--text);
    font: inherit;
    font-size: var(--fs-xs);
    text-align: start;
    cursor: pointer;
  }
  .mo-name:hover {
    background: var(--hover);
  }
  .mo-n {
    flex: 1;
    min-inline-size: 0;
    display: flex;
    white-space: pre;
  }
  .mo-h {
    min-inline-size: 0;
    overflow: hidden;
    text-overflow: ellipsis;
  }
  .mo-t {
    flex-shrink: 0;
  }
  .mo-c {
    font-family: var(--font-mono);
    color: var(--text-dim);
    font-variant-numeric: tabular-nums;
  }
  @media (pointer: coarse) {
    .mo-select,
    .mo-filter {
      min-block-size: 40px;
      font-size: 16px; /* ui-guards: allow */
    }
    .mo-name {
      min-block-size: 40px;
    }
    .mo-row :global(.icon-btn) {
      min-inline-size: 40px;
      min-block-size: 40px;
    }
  }
</style>
