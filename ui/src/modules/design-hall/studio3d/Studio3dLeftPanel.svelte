<script lang="ts">
  // Extracted left rail for Studio3D: scene hierarchy (scene3d), GLB/scene
  // sub-mesh outliner + poly-budget meter, environment preset, named camera views.
  import Icon from '../../../lib/components/Icon.svelte';
  import { rowMenu } from '../../../lib/rowMenu';
  import {
    ENV_PRESETS,
    Hierarchy,
    setEnvironment,
    type EnvPresetId,
    type MeshTopologyStats,
    type Scene3dCameraPreset,
    type Scene3dDoc,
    type SubMeshInfo,
  } from '../../../lib/design/scene3d';
  import MeshOutliner from './MeshOutliner.svelte';
  import type { BrandKit } from './sources';

  interface Props {
    doc: Scene3dDoc;
    isModelArtifact: boolean;
    readonly: boolean;
    canComment: boolean;
    selectedId: string | null;
    modelEnvPreset: EnvPresetId;
    subMeshes: SubMeshInfo[];
    topologyStats: MeshTopologyStats;
    activeMeshUid: string | null;
    kit: BrandKit | null;
    onselectid: (id: string | null) => void;
    onedit: (next: Scene3dDoc) => void;
    onmodelenv: (preset: EnvPresetId) => void;
    onimportglb: () => void;
    onfocusmesh: (uid: string) => void;
    ontogglevis: (uid: string) => void;
    oncommentmesh: (m: SubMeshInfo) => void;
    onsaveview: () => void;
    ongotoview: (cam: Scene3dCameraPreset) => void;
    onviewmenu: (e: MouseEvent, id: string) => void;
  }
  let {
    doc,
    isModelArtifact,
    readonly,
    canComment,
    selectedId,
    modelEnvPreset,
    subMeshes,
    topologyStats,
    activeMeshUid,
    kit,
    onselectid,
    onedit,
    onmodelenv,
    onimportglb,
    onfocusmesh,
    ontogglevis,
    oncommentmesh,
    onsaveview,
    ongotoview,
    onviewmenu,
  }: Props = $props();

  const env = $derived(doc.environment);
</script>

<aside class="left" id="s3d-scene-tools" aria-label="Scene" data-testid="s3d-left">
  {#if !isModelArtifact}
    <div class="hier">
      <Hierarchy
        {doc}
        bind:selectedId={() => selectedId, onselectid}
        onchange={onedit}
        {readonly}
        onimportGlb={onimportglb}
      />
    </div>
  {/if}

  <MeshOutliner
    meshes={subMeshes}
    stats={topologyStats}
    {activeMeshUid}
    {canComment}
    onfocus={onfocusmesh}
    ontogglevis={ontogglevis}
    oncommentmesh={oncommentmesh}
  />

  <section class="block" aria-label="Environment">
    <div class="block-head">
      <span class="k">Environment</span>
    </div>
    <div class="env-row">
      <Icon name="globe" size={13} />
      <select
        class="env-select"
        value={isModelArtifact ? modelEnvPreset : (env?.preset ?? 'none')}
        disabled={!isModelArtifact && readonly}
        aria-label="Environment preset"
        data-testid="s3d-env"
        onchange={(e) => {
          const v = (e.currentTarget as HTMLSelectElement).value as EnvPresetId;
          if (isModelArtifact) onmodelenv(v);
          else onedit(setEnvironment(doc, { preset: v }));
        }}
      >
        {#each ENV_PRESETS as p (p.id)}<option value={p.id}>{p.label}</option>{/each}
      </select>
    </div>
    {#if !isModelArtifact && env && env.preset !== 'none'}
      <label class="env-row check">
        <input
          type="checkbox"
          checked={env.background !== false}
          disabled={readonly}
          onchange={(e) => onedit(setEnvironment(doc, { background: (e.currentTarget as HTMLInputElement).checked }))}
        />
        <span>Show as backdrop</span>
      </label>
    {/if}
  </section>

  {#if !isModelArtifact}
    <section class="block" aria-label="Views">
      <div class="block-head">
        <span class="k">Views</span>
        {#if !readonly}
          <button type="button" class="icon-btn" aria-label="Save the current view" title="Save the current view" onclick={onsaveview}>
            <Icon name="plus" size={13} />
          </button>
        {/if}
      </div>
      {#if doc.cameras?.length}
        <ul class="views">
          {#each doc.cameras as c (c.id)}
            <li>
              <button
                type="button"
                use:rowMenu
                class="view"
                onclick={() => { onselectid(null); ongotoview(c); }}
                oncontextmenu={(e) => { e.preventDefault(); onviewmenu(e, c.id); }}
                title="#view:{c.id}"
              >
                <Icon name="eye" size={12} /> <span>{c.name ?? c.id}</span>
              </button>
              <button type="button" class="icon-btn" aria-label="View options" title="View options" onclick={(e) => onviewmenu(e, c.id)}>
                <Icon name="more" size={12} />
              </button>
            </li>
          {/each}
        </ul>
      {:else}
        <p class="dim">Frame a shot, then + to save it (e.g. “Hero angle”) — site embeds can use it.</p>
      {/if}
    </section>

    <section class="block summary-card" aria-label="Scene summary">
      <span class="k">Scene</span>
      <p>
        {doc.objects.length} objects · {doc.lights.length} lights{doc.states?.length ? ` · ${doc.states.length} states` : ''}.
        {#if kit}Materials can use <strong>{kit.label}</strong>.{:else}No brand kit in this project yet — brand colors appear once one exists.{/if}
      </p>
    </section>
  {/if}
</aside>

<style>
  .left {
    min-block-size: 0;
    background: var(--surface);
    display: flex;
    flex-direction: column;
    border-inline-end: 1px solid var(--border);
    overflow-y: auto;
  }
  .hier {
    min-block-size: 200px;
    flex: 1 1 auto;
    display: flex;
    flex-direction: column;
  }
  .hier > :global(*) {
    flex: 1;
  }
  /* Touch: the pane is the one scroll area (a list scrolling inside a scrolling pane traps the
     finger, and the hierarchy was a three-row window); its lists take their full height. */
  @media (pointer: coarse) {
    .hier {
      flex: none;
      min-block-size: 0;
    }
    .hier :global(.s3d-hier-body),
    .left :global(.mo-list) {
      overflow: visible;
      max-block-size: none;
    }
    .left :global(.mo-list) {
      overflow-x: clip;
    }
  }
  .block {
    padding: 10px 12px;
    border-block-start: 1px solid var(--border);
    display: flex;
    flex-direction: column;
    gap: 6px;
  }
  .block.summary-card {
    margin: 10px 12px 12px;
    padding: 10px 12px;
    border: 1px solid var(--border);
    border-radius: var(--radius-m);
    background: var(--surface-2);
  }
  .block.summary-card p {
    margin: 0;
    font-size: var(--fs-s);
    color: var(--text-dim);
    line-height: 1.45;
  }
  .block-head {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 8px;
  }
  .k {
    font-size: var(--fs-xs);
    font-weight: 600;
    letter-spacing: 0.06em;
    text-transform: uppercase;
    color: var(--text-dim);
  }
  .env-row {
    display: flex;
    align-items: center;
    gap: 8px;
    font-size: var(--fs-s);
    color: var(--text-dim);
  }
  .env-row.check {
    cursor: pointer;
  }
  .env-row input {
    accent-color: var(--accent);
  }
  .env-select {
    flex: 1;
    min-inline-size: 0;
    font: inherit;
    font-size: var(--fs-s);
    color: var(--text);
    padding: 4px 6px;
    border: 1px solid var(--border);
    border-radius: var(--radius-s);
    background: var(--bg);
  }
  .views {
    list-style: none;
    margin: 0;
    padding: 0;
    display: flex;
    flex-direction: column;
    gap: 2px;
  }
  .views li {
    display: flex;
    align-items: center;
    gap: 2px;
  }
  .view {
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
    font-size: var(--fs-s);
    cursor: pointer;
    text-align: start;
  }
  .view:hover {
    background: var(--hover);
  }
  .view span {
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .dim {
    margin: 0;
    font-size: var(--fs-s);
    color: var(--text-dim);
  }
</style>
