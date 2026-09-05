<script lang="ts">
  // ChildrenBoard — the epic Overview's "Children" board (design §3.2): one column
  // per folder (unfiled first), a card per child story / doc, and a roll-up of
  // counts across the whole epic. Clicking a card opens the child; the epic's
  // Design tab is where all children's artifacts are reviewed together.
  import Icon from '../../lib/components/Icon.svelte';
  import { product } from '../../lib/stores/product.svelte';
  import type { ProductStory } from './types';

  interface Props {
    epicId: string;
    onaddchild?: (e: MouseEvent) => void;
  }
  const { epicId, onaddchild }: Props = $props();

  const children = $derived<ProductStory[]>(product.childrenOf(epicId));
  /** Folder → children (unfiled `''` first, then A→Z). */
  const folders = $derived.by(() => {
    const map = new Map<string, ProductStory[]>();
    for (const c of children) {
      const list = map.get(c.folder ?? '') ?? [];
      list.push(c);
      map.set(c.folder ?? '', list);
    }
    return [...map.entries()]
      .sort(([a], [b]) => (a === '' ? -1 : b === '' ? 1 : a.localeCompare(b, undefined, { sensitivity: 'base' })))
      .map(([name, list]) => ({ name, list }));
  });
  const rollup = $derived.by(() => {
    const stages: Record<string, number> = {};
    let docs = 0;
    for (const c of children) {
      if (c.tree_kind === 'doc') docs++;
      else stages[c.stage] = (stages[c.stage] ?? 0) + 1;
    }
    return { total: children.length, docs, stories: children.length - docs, stages, folders: folders.filter((f) => f.name).length };
  });

  function stageColor(stage: string): string {
    switch (stage) {
      case 'draft': return 'stage-draft';
      case 'review': return 'stage-review';
      case 'approved': return 'stage-approved';
      case 'done': return 'stage-done';
      default: return 'stage-other';
    }
  }
</script>

<section class="children-board">
  <div class="cb-head">
    <span class="cb-title"><Icon name="folder" size={13} /> Children</span>
    <span class="cb-rollup">
      {rollup.total} total · {rollup.stories} {rollup.stories === 1 ? 'story' : 'stories'} · {rollup.docs} {rollup.docs === 1 ? 'doc' : 'docs'}
      {#if rollup.folders}· {rollup.folders} {rollup.folders === 1 ? 'folder' : 'folders'}{/if}
      {#each Object.entries(rollup.stages) as [st, n] (st)}
        <span class="stage-badge {stageColor(st)}">{n} {st}</span>
      {/each}
    </span>
    {#if onaddchild}
      <button class="p-btn" onclick={onaddchild}><Icon name="plus" size={12} /> Add child</button>
    {/if}
  </div>
  {#if children.length === 0}
    <p class="cb-empty">No children yet. Swarm agents file their drafts here (<code>otto-product --folder Design</code>), or use <strong>Add child</strong>.</p>
  {:else}
    <div class="cb-columns">
      {#each folders as f (f.name)}
        <div class="cb-col">
          <div class="cb-col-head">
            <Icon name="folder" size={11} /> {f.name || 'Unfiled'} <span class="cb-count">{f.list.length}</span>
          </div>
          {#each f.list as c (c.id)}
            <button class="cb-card" onclick={() => void product.select(c.id)} title={c.source_key}>
              <span class="cb-card-title">{c.title}</span>
              <span class="cb-card-meta">
                {#if c.tree_kind === 'doc'}
                  <span class="doc-badge">DOC</span>
                {:else}
                  <span class="stage-badge {stageColor(c.stage)}">{c.stage}</span>
                {/if}
                {#if c.source_kind !== 'draft'}<span class="mono key">{c.source_key}</span>{/if}
              </span>
            </button>
          {/each}
        </div>
      {/each}
    </div>
  {/if}
</section>

<style>
  .children-board {
    border: 1px solid var(--border);
    border-radius: var(--radius-m, 8px);
    padding: 10px 12px 12px;
    margin-top: 14px;
    display: flex;
    flex-direction: column;
    gap: 10px;
  }
  .cb-head {
    display: flex;
    align-items: center;
    gap: 10px;
    flex-wrap: wrap;
  }
  .cb-title {
    display: inline-flex;
    align-items: center;
    gap: 6px;
    font-size: 12.5px;
    font-weight: 600;
  }
  .cb-rollup {
    display: inline-flex;
    align-items: center;
    gap: 6px;
    flex-wrap: wrap;
    font-size: 11px;
    color: var(--text-dim);
    flex: 1;
    min-width: 0;
  }
  .cb-empty {
    margin: 0;
    font-size: 12px;
    color: var(--text-dim);
    line-height: 1.5;
  }
  .cb-empty code {
    font-family: var(--font-mono, monospace);
    font-size: 11px;
  }
  .cb-columns {
    display: grid;
    grid-template-columns: repeat(auto-fill, minmax(200px, 1fr));
    gap: 10px;
  }
  .cb-col {
    display: flex;
    flex-direction: column;
    gap: 6px;
    background: color-mix(in srgb, var(--text-dim) 6%, transparent);
    border-radius: var(--radius-s);
    padding: 8px;
    min-width: 0;
  }
  .cb-col-head {
    display: flex;
    align-items: center;
    gap: 5px;
    font-size: 10.5px;
    font-weight: 700;
    text-transform: uppercase;
    letter-spacing: 0.05em;
    color: var(--text-dim);
  }
  .cb-count {
    margin-inline-start: auto;
    font-weight: 600;
  }
  .cb-card {
    display: flex;
    flex-direction: column;
    gap: 5px;
    padding: 8px 10px;
    border: 1px solid var(--border);
    border-radius: var(--radius-s);
    background: var(--surface);
    color: var(--text);
    cursor: pointer;
    text-align: start;
    min-width: 0;
  }
  .cb-card:hover {
    border-color: var(--accent);
  }
  .cb-card-title {
    font-size: 12.5px;
    font-weight: 500;
    line-height: 1.3;
    display: -webkit-box;
    -webkit-line-clamp: 2;
    -webkit-box-orient: vertical;
    line-clamp: 2;
    overflow: hidden;
  }
  .cb-card-meta {
    display: flex;
    align-items: center;
    gap: 6px;
    font-size: 10.5px;
    color: var(--text-dim);
  }
  .stage-badge {
    font-size: 9.5px;
    font-weight: 600;
    text-transform: uppercase;
    letter-spacing: 0.05em;
    padding: 1px 6px;
    border-radius: 999px;
  }
  .stage-draft { background: color-mix(in srgb, var(--text-dim) 18%, transparent); color: var(--text-dim); }
  .stage-review { background: color-mix(in srgb, #f59e0b 18%, transparent); color: #b45309; }
  .stage-approved { background: color-mix(in srgb, var(--status-working) 18%, transparent); color: var(--status-working); }
  .stage-done { background: color-mix(in srgb, var(--accent) 18%, transparent); color: var(--accent); }
  .stage-other { background: color-mix(in srgb, var(--text-dim) 12%, transparent); color: var(--text-dim); }
  .doc-badge {
    font-size: 9px;
    font-weight: 700;
    letter-spacing: 0.08em;
    padding: 1px 5px;
    border-radius: 999px;
    background: color-mix(in srgb, var(--text-dim) 18%, transparent);
    color: var(--text-dim);
  }
  .mono {
    font-family: var(--font-mono, monospace);
  }
</style>
