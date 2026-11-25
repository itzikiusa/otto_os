<script lang="ts">
  // The DB Watch line in the Debug panel: what the last stop wrote, readable
  // without leaving the Debug tab — "DB · session_info +1 · session_route ~1 ·
  // Open". A table opens DB Watch on it. Shown only while a watch is armed.
  import Icon from '../../../lib/components/Icon.svelte';
  import { plural } from '../../../lib/plural';
  import { ide } from '../ideStore.svelte';
  import { countsShort } from './ideDbWatchModel';
  import { ideDbWatch } from './ideDbWatchState.svelte';

  const items = $derived(ideDbWatch.read?.items ?? []);
  const changed = $derived(items.filter((i) => i.error || countsShort(i)));
  const waiting = $derived(!ideDbWatch.read || ideDbWatch.read.from_seq === null);

  function open(object = ''): void {
    ideDbWatch.focusObject = object;
    ide.openBottomTab('dbwatch');
  }
</script>

<div class="dbs" data-testid="ide-dbw-strip">
  <Icon name="db" size={12} />
  <span class="dbs-label">DB</span>
  {#if waiting}
    <span class="dbs-quiet">watching {plural(ideDbWatch.watch?.items.length ?? 0, 'table')}{ideDbWatch.watch?.busy ? '…' : ''}</span>
  {:else if changed.length === 0}
    <span class="dbs-quiet">no changes{ideDbWatch.compare === 'start' ? ' since the start' : ' since the last stop'}</span>
  {:else}
    {#each changed as it (it.object)}
      <button type="button" class="dbs-item mono" dir="ltr" title="Show {it.object} in DB Watch" onclick={() => open(it.object)}>
        {it.object}
        {#if it.error}<span class="dbs-err">error</span>{:else}<span class="dbs-counts">{countsShort(it)}</span>{/if}
      </button>
    {/each}
  {/if}
  <button type="button" class="btn small ghost dbs-open" onclick={() => open()} data-testid="ide-dbw-strip-open">Open</button>
</div>

<style>
  .dbs {
    display: flex; align-items: center; gap: var(--sp-3);
    block-size: 26px; padding-inline: var(--sp-4) var(--sp-2); flex-shrink: 0; overflow: hidden;
    border-block-end: 1px solid var(--separator); background: var(--surface-2); font-size: var(--fs-xs); color: var(--text-dim);
  }
  .dbs > :global(svg) { flex-shrink: 0; }
  .dbs-label { font-weight: 600; color: var(--text); }
  .dbs-quiet { white-space: nowrap; overflow: hidden; text-overflow: ellipsis; min-inline-size: 0; }
  .dbs-item { display: inline-flex; gap: var(--sp-2); border: 0; background: none; padding: 0; color: var(--text); font-size: var(--fs-xs); white-space: nowrap; cursor: pointer; }
  .dbs-item:hover { color: var(--accent-text); text-decoration: underline; }
  .dbs-counts { color: var(--warning); }
  .dbs-err { color: var(--danger); }
  .dbs-open { margin-inline-start: auto; block-size: 20px; flex-shrink: 0; }
</style>
