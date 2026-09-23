<script lang="ts">
  // "Linked to Product": the epic tree of stories that designs `implement`,
  // with per-studio artifact counts. A story row opens its designs in the Hall.
  import Icon from '../../lib/components/Icon.svelte';
  import StudioBadge from './StudioBadge.svelte';
  import { countTotal, epicTree, studioInfo } from './model';
  import { library } from './library.svelte';
  import type { DesignStudio } from '../../lib/api/types';

  const rows = $derived(epicTree(library.hits, Object.values(library.stories)));
  let open = $state<Record<string, boolean>>({});
  const isOpen = (id: string, i: number) => open[id] ?? i === 0;

  function toggle(id: string, i: number): void {
    open = { ...open, [id]: !isOpen(id, i) };
  }

  const entries = (c: Partial<Record<DesignStudio, number>>) =>
    Object.entries(c) as [DesignStudio, number][];
</script>

{#if rows.length === 0}
  <p class="empty">
    No design implements a story yet. Link a design to its story from the artifact’s Links panel, or
    create one from a story’s Design tab in Product.
  </p>
{:else}
  <div class="tree card" data-testid="design-linked-product">
    {#each rows as r, i (r.story.id)}
      {@const kids = r.children.length}
      <div class="epic">
        {#if kids}
          <button class="icon-btn" onclick={() => toggle(r.story.id, i)} aria-expanded={isOpen(r.story.id, i)}
            aria-label={`${isOpen(r.story.id, i) ? 'Collapse' : 'Expand'} ${r.story.source_key}`}
            title={`${isOpen(r.story.id, i) ? 'Collapse' : 'Expand'} ${r.story.source_key}`}>
            <Icon name={isOpen(r.story.id, i) ? 'chevronDown' : 'chevronRight'} size={14} />
          </button>
        {:else}
          <span class="gap"></span>
        {/if}
        <a class="row-link" href={`#/design/story/${encodeURIComponent(r.story.id)}`}>
          <span class="key">{r.story.source_key}</span>
          <span class="title">{r.story.title}</span>
        </a>
        <span class="counts">
          {#each entries(r.own) as [s, n] (s)}
            <span class="count" title={`${n} in ${studioInfo(s).name}`}><StudioBadge studio={s} /> {n}</span>
          {/each}
        </span>
        <span class="meta">
          {#if r.childCount}{r.childCount} stor{r.childCount === 1 ? 'y' : 'ies'} · {kids} with designs{:else}{countTotal(r.own)} design{countTotal(r.own) === 1 ? '' : 's'}{/if}
        </span>
      </div>
      {#if kids && isOpen(r.story.id, i)}
        {#each r.children as c (c.story.id)}
          <a class="child" href={`#/design/story/${encodeURIComponent(c.story.id)}`}>
            <span class="elbow" aria-hidden="true"></span>
            <span class="key">{c.story.source_key}</span>
            <span class="title">{c.story.title}</span>
            <span class="counts">
              {#each entries(c.counts) as [s, n] (s)}
                <span class="count" title={`${n} in ${studioInfo(s).name}`}><StudioBadge studio={s} /> {studioInfo(s).name} {n}</span>
              {/each}
            </span>
          </a>
        {/each}
      {/if}
    {/each}
  </div>
{/if}

<style>
  .empty {
    margin: 0;
    font-size: var(--fs-s);
    color: var(--text-dim);
  }
  .tree {
    padding: 6px 8px;
  }
  .epic,
  .child {
    display: flex;
    align-items: center;
    gap: 8px;
    min-height: 32px;
    min-width: 0;
  }
  .epic + .epic {
    border-block-start: 1px solid var(--border);
  }
  .gap {
    width: 24px;
    flex: none;
  }
  .row-link {
    display: flex;
    align-items: center;
    gap: 8px;
    min-width: 0;
    flex: 1;
    color: var(--text);
    text-decoration: none;
    font-weight: 600;
  }
  .child {
    padding-inline-start: 24px;
    color: var(--text);
    text-decoration: none;
    border-radius: var(--radius-s);
  }
  .row-link:hover .title,
  .child:hover .title {
    color: var(--accent-text);
  }
  .elbow {
    width: 10px;
    height: 14px;
    margin-block-start: -12px;
    border-inline-start: 1px solid var(--border-strong);
    border-block-end: 1px solid var(--border-strong);
    border-end-start-radius: 4px;
    flex: none;
  }
  .key {
    font-family: var(--font-mono);
    font-size: var(--fs-xs);
    color: var(--text-dim);
    padding: 1px 6px;
    border-radius: var(--radius-s);
    background: var(--surface-2);
    border: 1px solid var(--border);
    flex: none;
  }
  .title {
    min-width: 0;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .child .title {
    flex: 1;
  }
  .counts {
    display: inline-flex;
    gap: 6px;
    flex: none;
  }
  .count {
    display: inline-flex;
    align-items: center;
    gap: 4px;
    height: 22px;
    padding: 0 6px 0 3px;
    border: 1px solid var(--border);
    border-radius: var(--radius-s);
    font-size: var(--fs-xs);
    color: var(--text);
    font-variant-numeric: tabular-nums;
  }
  .meta {
    font-size: var(--fs-xs);
    color: var(--text-dim);
    flex: none;
  }
  @media (max-width: 640px) {
    .meta,
    .child .counts .count :global(.name) {
      display: none;
    }
  }
</style>
