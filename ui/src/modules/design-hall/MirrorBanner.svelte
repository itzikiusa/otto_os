<script lang="ts">
  // The one banner of a design that mirrors a Product story attachment or a
  // Canvas board. A mirror is read-only here (the daemon refuses writes with
  // 409 `mirrored`) until a person chooses "Edit here", which adopts it
  // (`POST …/adopt`) and turns editing on in place — a newer source then
  // arrives as a side version to accept or keep yours. A source that vanished
  // (`meta.source_missing`) only gets a quiet note.
  import Icon from '../../lib/components/Icon.svelte';
  import { announceShown } from '../../lib/announce.svelte';
  import type { DesignArtifact } from '../../lib/api/types';
  import { isMirrored, mirrorSourceLabel } from './versionRows';
  import { originOf } from './nav';
  import { mirrorLabel } from './designGrid';

  interface Props {
    artifact: DesignArtifact;
    /** The person may edit designs (else the banner explains, without the button). */
    canEdit: boolean;
    /** A write was just refused as `mirrored` (the meta may still say adopted). */
    refused?: boolean;
    adopting?: boolean;
    onadopt: () => void;
  }
  let { artifact, canEdit, refused = false, adopting = false, onadopt }: Props = $props();

  const mirrored = $derived(isMirrored(artifact) || refused);
  const from = $derived(mirrorSourceLabel(artifact.source_kind));
  /** The lobby card's badge words ("From a story" / "From Canvas"). */
  const badge = $derived(mirrorLabel(artifact.source_kind)?.label ?? 'Mirrored');
  const origin = $derived(originOf(artifact));
  const missing = $derived(artifact.meta?.source_missing === true);
</script>

{#if mirrored}
  <div class="banner" data-testid="design-mirror-banner" use:announceShown={`This design mirrors ${from}.`}>
    <Icon name="link" size={14} />
    <span class="txt">
      <strong>{badge} · This design mirrors {from}.</strong>
      {#if missing}
        <span class="dim">The original is gone; its versions are kept here.</span>
      {:else}
        <span class="dim">It updates when the original changes. Edit here to take it over — later changes there arrive as a version you can accept.</span>
      {/if}
    </span>
    {#if origin && !missing}<button class="btn small ghost" onclick={origin.open}>{origin.label}</button>{/if}
    {#if canEdit}
      <button class="btn small primary" onclick={onadopt} disabled={adopting} data-testid="design-adopt">{adopting ? 'Taking over…' : 'Edit here'}</button>
    {:else}
      <span class="dim"><Icon name="lock" size={12} /> Read-only</span>
    {/if}
  </div>
{:else if missing}
  <p class="quiet" data-testid="design-source-missing">
    <Icon name="info" size={12} /> The {artifact.source_kind === 'canvas_scene' ? 'Canvas board' : 'story attachment'} this design came from is gone. Nothing changes here.
  </p>
{/if}

<style>
  .banner {
    display: flex;
    align-items: center;
    flex-wrap: wrap;
    gap: 8px 10px;
    padding: 8px 12px;
    border-block-end: 1px solid var(--border);
    background: var(--info-soft);
    color: var(--text);
    font-size: var(--fs-s);
  }
  .banner > :global(svg) {
    color: var(--info);
    flex: none;
  }
  .txt {
    flex: 1 1 260px;
    min-width: 0;
    display: flex;
    flex-direction: column;
    gap: 2px;
  }
  .dim {
    color: var(--text-dim);
    display: inline-flex;
    align-items: center;
    gap: 4px;
  }
  .quiet {
    margin: 0;
    padding: 6px 12px;
    border-block-end: 1px solid var(--border);
    display: flex;
    align-items: center;
    gap: 6px;
    font-size: var(--fs-xs);
    color: var(--text-dim);
    background: var(--bg);
  }
</style>
