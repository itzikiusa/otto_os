<script lang="ts">
  // The Generate dialog's provider radios: the set-up providers above the form
  // when there is more than Otto Forge, and the Advanced list (with the Otto +
  // Blender agent turn on a generate) under it.
  import Badge from '../../../lib/components/Badge.svelte';
  import type { Media3dProvider } from '../../../lib/api/types';
  import { licenseWarningFor } from './providers';

  interface Props {
    providers: Media3dProvider[];
    selectedId: string;
    legend: string;
    /** Adds the Local: Otto + Blender MCP radio with this id. */
    localId?: string | null;
  }
  let { providers, selectedId = $bindable(), legend, localId = null }: Props = $props();
</script>

<fieldset class="g3-provs">
  <legend class="sr-only">{legend}</legend>
  {#each providers as p (p.id)}
    {@const lic = licenseWarningFor(p)}
    <label class="g3-prov" class:on={selectedId === p.id} class:off={!p.ready || !p.enabled}>
      <input type="radio" name="gen3d-prov" value={p.id} bind:group={selectedId} />
      <span class="g3-pbody">
        <span class="g3-pname">
          {p.name}
          <Badge tone={p.ready && p.enabled ? 'ok' : 'warn'} label={p.ready && p.enabled ? 'Ready' : 'Configure'} />
          {#if lic}<Badge tone="warn" label="Territory restriction" />{/if}
        </span>
        <span class="g3-pnote">{p.status_detail}</span>
      </span>
    </label>
  {/each}
  {#if localId}
    <label class="g3-prov" class:on={selectedId === localId}>
      <input type="radio" name="gen3d-prov" value={localId} bind:group={selectedId} />
      <span class="g3-pbody">
        <span class="g3-pname">Local: Otto + Blender MCP <Badge label="Built-in" /></span>
        <span class="g3-pnote">Runs an Otto agent turn on this scene; Blender MCP tool calls ask first.</span>
      </span>
    </label>
  {/if}
</fieldset>

<style>
  .g3-provs {
    border: 0;
    margin: 0;
    padding: 0;
    display: flex;
    flex-direction: column;
    gap: 6px;
  }
  .g3-prov {
    display: flex;
    align-items: flex-start;
    gap: 10px;
    padding: 8px 10px;
    border: 1px solid var(--border);
    border-radius: var(--radius-m);
    cursor: pointer;
  }
  .g3-prov.on {
    border-color: var(--accent-line-strong);
    background: var(--accent-soft);
  }
  .g3-prov.off {
    color: var(--text-dim);
  }
  .g3-pbody {
    display: flex;
    flex-direction: column;
    gap: 2px;
    min-inline-size: 0;
  }
  .g3-pname {
    display: inline-flex;
    align-items: center;
    flex-wrap: wrap;
    gap: 6px;
    font-size: var(--fs-m);
    font-weight: 600;
  }
  .g3-pnote {
    margin: 0;
    font-size: var(--fs-xs);
    color: var(--text-dim);
  }
</style>
