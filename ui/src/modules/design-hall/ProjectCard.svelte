<script lang="ts">
  // A project in the lobby grid: a 2×2 mosaic of its newest artifacts, the
  // bound epic, counts, and a one-line review summary. Links to the project.
  import Icon from '../../lib/components/Icon.svelte';
  import { rel } from '../../lib/stores/now.svelte';
  import type { DesignArtifact, DesignProject } from '../../lib/api/types';
  import ArtifactThumb from './ArtifactThumb.svelte';
  import { projectStats, projectSummary } from './model';

  interface Props {
    project: DesignProject;
    artifacts: DesignArtifact[];
    /** "LOY-120 · Loyalty relaunch" when the epic is known. */
    epicLabel?: string | null;
    /** Render the mosaic's designs live when they have no stored thumbnail. */
    live?: boolean;
  }
  let { project, artifacts, epicLabel = null, live = false }: Props = $props();
  const stats = $derived(projectStats(project, artifacts));
  const summary = $derived(projectSummary(stats));
</script>

<a class="pcard" href={`#/design/p/${encodeURIComponent(project.id)}`} data-testid="design-project-card">
  <div class="mosaic" aria-hidden="true">
    {#each [0, 1, 2, 3] as i (i)}
      <div class="cell">
        {#if stats.mosaic[i]}<ArtifactThumb artifact={stats.mosaic[i]} {live} compact />{/if}
      </div>
    {/each}
  </div>
  <div class="body">
    <div class="name" title={project.name}>{project.name}</div>
    {#if epicLabel}
      <span class="chip epic" title={epicLabel}><Icon name="ticket" size={12} /> <span class="epic-t">{epicLabel}</span></span>
    {/if}
    <div class="meta">
      {stats.artifacts} artifact{stats.artifacts === 1 ? '' : 's'} · {stats.studios.length}
      studio{stats.studios.length === 1 ? '' : 's'} · updated {rel(stats.updatedAt)}
    </div>
    <div class="summary tone-{summary.tone}">{summary.text}</div>
  </div>
</a>

<style>
  .pcard {
    display: flex;
    gap: 14px;
    padding: 12px;
    min-width: 0;
    background: var(--surface);
    border: 1px solid var(--border);
    border-radius: var(--radius-m);
    color: var(--text);
    text-decoration: none;
    transition: border-color 130ms ease-out;
  }
  .pcard:hover {
    border-color: var(--border-strong);
  }
  .pcard:focus-visible {
    outline: 2px solid color-mix(in srgb, var(--accent) 70%, transparent);
    outline-offset: 1px;
  }
  .mosaic {
    flex: none;
    width: 132px;
    height: 108px;
    display: grid;
    grid-template-columns: 1fr 1fr;
    grid-template-rows: 1fr 1fr;
    gap: 2px;
    border-radius: var(--radius-s);
    overflow: hidden;
    background: var(--border);
  }
  .cell {
    background: var(--surface-2);
    min-width: 0;
    min-height: 0;
  }
  .body {
    flex: 1;
    min-width: 0;
    display: flex;
    flex-direction: column;
    align-items: flex-start;
    gap: 6px;
  }
  .name {
    max-width: 100%;
    font-weight: 600;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  /* .chip is inline-flex: ellipsis only works on the label's own box, so a
     long "KEY · epic title" was cut mid-glyph at the card edge. */
  .epic {
    max-width: 100%;
    overflow: hidden;
  }
  .epic-t {
    min-width: 0;
    overflow: hidden;
    text-overflow: ellipsis;
  }
  .meta {
    font-size: var(--fs-xs);
    color: var(--text-dim);
  }
  .summary {
    margin-top: auto;
    align-self: flex-end;
    font-size: var(--fs-xs);
    font-weight: 500;
    color: var(--text-dim);
  }
  .tone-warn {
    color: var(--warning);
  }
  .tone-ok {
    color: var(--success);
  }
  .tone-info {
    color: var(--info);
  }
</style>
