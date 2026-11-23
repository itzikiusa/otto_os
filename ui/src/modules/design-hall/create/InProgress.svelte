<script lang="ts">
  // "In progress": the creations being planned, waiting or made right now
  // (and finished ones not opened yet), one quiet row each — its picture or
  // icon, its name and where it is ("Planning", "Waiting, 1 ahead", "Making 3
  // of 15", "Ready to look"). A tap opens it. Shown only when there is one.
  import { untrack } from 'svelte';
  import Icon from '../../../lib/components/Icon.svelte';
  import { router } from '../../../lib/router.svelte';
  import { ws } from '../../../lib/stores/workspace.svelte';
  import ItemThumb from './ItemThumb.svelte';
  import { createStore } from './createStore.svelte';
  import { createHref } from './plan';
  import { firstPicture, stripState } from './progress';

  const list = $derived(createStore.inProgress());
  const sound = $derived(createStore.soundReady === true);

  $effect(() => {
    const id = ws.currentId;
    untrack(() => {
      if (id) void createStore.loadInProgress();
      if (createStore.soundReady === null) void createStore.loadSoundEngine();
    });
  });

</script>

{#if list.length}
  <section class="ip" aria-labelledby="ip-h" data-testid="create-in-progress">
    <h2 class="ip-h" id="ip-h">In progress</h2>
    <ul class="ip-list">
      {#each list as p (p.id)}
        {@const pic = firstPicture(p)}
        <li>
          <button type="button" class="ip-row" onclick={() => router.go(createHref('p', p.id))} data-testid="create-in-progress-item" data-production={p.id}>
            {#if pic}
              <ItemThumb artifactId={pic.artifact} preview={pic.preview} icon="layers" size={44} />
            {:else}
              <span class="cr-tile ip-tile" aria-hidden="true"><Icon name={p.status === 'done' ? 'layers' : 'clock'} size={24} /></span>
            {/if}
            <span class="ip-body">
              <b>{p.plan?.title || p.description?.slice(0, 60)}</b>
              <small class:ready={p.status === 'done'} data-testid="create-in-progress-state">{stripState(p, sound)}</small>
            </span>
            <Icon name="chevronRight" size={16} />
          </button>
        </li>
      {/each}
    </ul>
  </section>
{/if}

<style>
  .ip {
    margin-block-end: var(--sp-4);
  }
  .ip-h {
    margin: 0 0 var(--sp-2);
    font-size: var(--fs-m);
    font-weight: 600;
    color: var(--text-dim);
  }
  .ip-list {
    list-style: none;
    margin: 0;
    padding: 0;
    display: grid;
    grid-template-columns: repeat(auto-fill, minmax(min(100%, 260px), 1fr));
    gap: var(--sp-2);
  }
  .ip-row {
    display: flex;
    align-items: center;
    gap: var(--sp-3);
    inline-size: 100%;
    min-block-size: var(--cr-hit);
    padding: var(--sp-2);
    border: 1px solid var(--border);
    border-radius: var(--radius-l);
    background: var(--surface);
    color: var(--text);
    font: inherit;
    text-align: start;
    cursor: pointer;
  }
  .ip-row:hover {
    background: var(--hover);
  }
  .ip-row:focus-visible {
    outline: 3px solid var(--accent-text);
    outline-offset: 2px;
  }
  .ip-row :global(.cr-tile),
  .ip-row .ip-tile {
    inline-size: 44px;
    block-size: 44px;
  }
  .ip-body {
    flex: 1;
    min-inline-size: 0;
    display: grid;
  }
  .ip-body b {
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    font-size: var(--fs-m);
  }
  .ip-body small {
    color: var(--text-dim);
    font-size: var(--fs-s);
  }
  .ip-body small.ready {
    color: var(--success);
  }
</style>
