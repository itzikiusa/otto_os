<script lang="ts">
  // Brand Kit → Used in: every design with a `uses_tokens` link to this kit
  // (from `POST …/brand/impact` without content), grouped by studio, with how
  // each one follows the kit (approved / latest / pinned) and which tokens it
  // names. Opening a row opens that design.
  import type { BrandConsumer, BrandImpactResp } from '../../../lib/api/types';
  import Icon from '../../../lib/components/Icon.svelte';
  import Skeleton from '../../../lib/components/Skeleton.svelte';
  import StudioBadge from '../StudioBadge.svelte';
  import StatusPill from '../StatusPill.svelte';
  import { openArtifact } from '../nav';
  import { studioInfo } from '../model';

  interface Props {
    usage: BrandImpactResp | null;
    loading: boolean;
    error: string | null;
    kitId: string;
    onretry: () => void;
  }
  let { usage, loading, error, kitId, onretry }: Props = $props();

  function follows(c: BrandConsumer): string {
    if (c.policy === 'pinned') return 'pinned — stays until updated';
    if (c.policy === 'follow_latest') return 'follows every save';
    return 'follows the approved kit';
  }
  function tokens(c: BrandConsumer): string {
    if (c.whole_kit) return c.scanned ? 'the whole kit' : 'the whole kit (not scanned)';
    return c.tokens.length === 1 ? c.tokens[0] : `${c.tokens.length} tokens`;
  }
</script>

<section id="brand-used" class="bsec" aria-labelledby="brand-used-h">
  <div class="sec-h">
    <h2 id="brand-used-h">Used in</h2>
    <span class="meta">Every studio reads these tokens</span>
  </div>

  <div class="card" data-testid="brand-used-in">
    {#if loading && !usage}
      <Skeleton rows={2} height={32} />
    {:else if error && !usage}
      <p class="err" role="alert"><Icon name="warning" size={14} /> Couldn’t load where this kit is used. <span class="dim">{error}</span> <button class="btn small" onclick={onretry}>Retry</button></p>
    {:else if usage && usage.artifact_count === 0}
      <p class="empty">
        Nothing uses this kit yet. A design uses it once it links the kit — a site’s <span class="mono">brand</span> field set to
        <span class="mono">otto://design/{kitId}</span>, or a <em>Uses tokens</em> link from the Links panel — and names tokens like
        <span class="mono">token:color.primary</span>.
      </p>
    {:else if usage}
      <div class="sum">
        <span class="badges">{#each usage.by_studio as s (s.studio)}<StudioBadge studio={s.studio} />{/each}</span>
        <b>{usage.artifact_count} {usage.artifact_count === 1 ? 'design' : 'designs'}</b>
        <span class="dim">in {usage.studio_count} {usage.studio_count === 1 ? 'studio' : 'studios'}{usage.hidden_count ? ` · ${usage.hidden_count} more in workspaces you can’t see` : ''}</span>
      </div>
      <ul class="list">
        {#each usage.consumers as c (c.artifact.id)}
          <li>
            <button class="row" onclick={() => openArtifact(c.artifact.id)} data-testid="brand-used-row" title="Open {c.artifact.title}">
              <StudioBadge studio={c.artifact.studio} />
              <span class="t">{c.artifact.title}</span>
              <span class="dim studio">{studioInfo(c.artifact.studio).name}</span>
              <StatusPill status={c.artifact.status} />
              <span class="chip">{follows(c)}</span>
              <span class="mono dim tok">{tokens(c)}</span>
              <Icon name="chevronRight" size={14} />
            </button>
          </li>
        {/each}
      </ul>
    {/if}
  </div>
</section>

<style>
  .bsec {
    display: flex;
    flex-direction: column;
    gap: 12px;
    scroll-margin-top: 12px;
  }
  .sec-h {
    display: flex;
    align-items: baseline;
    gap: 10px;
    flex-wrap: wrap;
  }
  h2 {
    margin: 0;
    font-size: var(--fs-l);
    font-weight: 600;
  }
  .meta,
  .dim {
    color: var(--text-dim);
    font-size: var(--fs-s);
  }
  .card {
    background: var(--surface);
    border: 1px solid var(--border);
    border-radius: var(--radius-m);
    padding: 12px 14px;
    display: flex;
    flex-direction: column;
    gap: 10px;
  }
  .sum {
    display: flex;
    align-items: center;
    gap: 10px;
    flex-wrap: wrap;
  }
  .badges {
    display: inline-flex;
    gap: 4px;
  }
  .list {
    list-style: none;
    margin: 0;
    padding: 0;
    display: flex;
    flex-direction: column;
  }
  .list li + li {
    border-block-start: 1px solid var(--border);
  }
  .row {
    width: 100%;
    display: flex;
    align-items: center;
    gap: 10px;
    padding: 8px 4px;
    background: transparent;
    border: 0;
    color: var(--text);
    font: inherit;
    text-align: start;
    cursor: pointer;
    border-radius: var(--radius-s);
    min-width: 0;
  }
  .row:hover {
    background: var(--hover);
  }
  .t {
    flex: 1;
    min-width: 0;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    font-weight: 500;
  }
  .tok {
    font-size: var(--fs-xs);
    max-width: 180px;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .empty,
  .err {
    margin: 0;
    font-size: var(--fs-s);
    color: var(--text-dim);
    line-height: 1.6;
  }
  .err {
    display: flex;
    align-items: center;
    gap: 8px;
    flex-wrap: wrap;
    color: var(--text);
  }
  .err > :global(svg) {
    color: var(--danger);
  }
  @media (max-width: 720px) {
    .studio,
    .tok {
      display: none;
    }
  }
</style>
