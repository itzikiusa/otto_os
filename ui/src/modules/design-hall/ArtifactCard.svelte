<script lang="ts">
  // One design as a card (Continue strip, project and studio pages): preview,
  // title + head version, studio + status, when it last changed, and link chips
  // (the stories it implements, how often it is referenced). The whole card is
  // a link to the artifact view.
  import Icon from '../../lib/components/Icon.svelte';
  import { rel } from '../../lib/stores/now.svelte';
  import type { DesignArtifact } from '../../lib/api/types';
  import ArtifactThumb from './ArtifactThumb.svelte';
  import StudioBadge from './StudioBadge.svelte';
  import StatusPill from './StatusPill.svelte';

  interface Props {
    artifact: DesignArtifact;
    /** Story keys this artifact implements ("LOY-142"). */
    stories?: string[];
    /** Incoming references (`DesignSearchHit.reference_count`). */
    referenceCount?: number;
    live?: boolean;
  }
  let { artifact, stories = [], referenceCount = 0, live = false }: Props = $props();
  const by = $derived(artifact.created_by_kind === 'agent' ? ' · drafted by Otto' : '');
</script>

<a class="acard" href={`#/design/a/${encodeURIComponent(artifact.id)}`} data-testid="design-artifact-card">
  <div class="pic"><ArtifactThumb {artifact} {live} /></div>
  <div class="body">
    <div class="line1">
      <span class="title" title={artifact.title}>{artifact.title}</span>
      {#if artifact.head_seq != null}<span class="ver">v{artifact.head_seq}</span>{/if}
    </div>
    <div class="line2">
      <StudioBadge studio={artifact.studio} showName />
      <StatusPill status={artifact.status} />
    </div>
    <div class="meta" title={new Date(artifact.updated_at).toLocaleString()}>
      edited {rel(artifact.updated_at)}{by}
    </div>
    {#if stories.length || referenceCount}
      <div class="chips">
        {#if referenceCount}
          <span class="chip"><Icon name="link" size={12} /> used in {referenceCount}</span>
        {/if}
        {#each stories.slice(0, 2) as key (key)}
          <span class="chip mono-chip"><Icon name="ticket" size={12} /> {key}</span>
        {/each}
      </div>
    {/if}
  </div>
</a>

<style>
  .acard {
    display: flex;
    flex-direction: column;
    min-width: 0;
    background: var(--surface);
    border: 1px solid var(--border);
    border-radius: var(--radius-m);
    overflow: hidden;
    color: var(--text);
    text-decoration: none;
    transition: border-color 130ms ease-out;
  }
  .acard:hover {
    border-color: var(--border-strong);
  }
  .acard:focus-visible {
    outline: 2px solid color-mix(in srgb, var(--accent) 70%, transparent);
    outline-offset: 1px;
  }
  .pic {
    height: 148px;
    border-block-end: 1px solid var(--border);
  }
  .body {
    display: flex;
    flex-direction: column;
    gap: 6px;
    padding: 10px 12px 12px;
    min-width: 0;
  }
  .line1 {
    display: flex;
    align-items: baseline;
    gap: 8px;
    min-width: 0;
  }
  .title {
    flex: 1;
    min-width: 0;
    font-weight: 600;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .ver {
    font-size: var(--fs-xs);
    color: var(--text-dim);
    font-variant-numeric: tabular-nums;
  }
  .line2 {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 8px;
    min-width: 0;
  }
  .meta {
    font-size: var(--fs-xs);
    color: var(--text-dim);
  }
  .chips {
    display: flex;
    flex-wrap: wrap;
    gap: 4px;
  }
  .mono-chip {
    font-family: var(--font-mono);
  }
</style>
