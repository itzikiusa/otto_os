<script lang="ts">
  // The results footer's sort chip. The header sort orders the LOADED rows in
  // memory; on a capped or paged result that is a slice of the table, so the
  // chip says "sorted N loaded rows" and — when the statement can be rewritten
  // (`server-sort.ts`) — offers "Sort on server", which re-runs it with an
  // ORDER BY. Click the chip to clear the sort.
  import Icon from '../../lib/components/Icon.svelte';

  interface Props {
    name: string;
    dir: 'asc' | 'desc';
    /** Loaded row count when the result is capped / paged, else null. */
    loaded: number | null;
    /** The ORDER BY rewrite, or null when the statement can't be rewritten. */
    serverSql: string | null;
    onclear: () => void;
    onserver: (sql: string) => void;
  }
  let { name, dir, loaded, serverSql, onclear, onserver }: Props = $props();
</script>

<button
  class="sort-chip"
  title={loaded !== null
    ? `Sorted the ${loaded.toLocaleString()} loaded rows by ${name}, not the whole table — click to clear`
    : `Clear sort on ${name}`}
  onclick={onclear}
>
  <Icon name={dir === 'asc' ? 'arrowUp' : 'arrowDown'} size={12} />
  <span class="sort-chip-name">{name}</span>
  {#if loaded !== null}<span class="sort-chip-scope">sorted {loaded.toLocaleString()} loaded rows</span>{/if}
  <Icon name="x" size={12} />
</button>
{#if loaded !== null && serverSql}
  <button
    class="sort-server"
    onclick={() => serverSql && onserver(serverSql)}
    title="Re-run the query with ORDER BY {name} {dir.toUpperCase()} so the server sorts every row before the limit"
  >Sort on server</button>
{/if}

<style>
  .sort-chip {
    display: inline-flex;
    align-items: center;
    gap: 4px;
    height: 16px;
    padding: 0 6px;
    border: 1px solid var(--accent-line);
    border-radius: 999px;
    font-size: var(--fs-xs);
    font-weight: 600;
    color: var(--accent-text);
    background: var(--accent-soft);
    cursor: pointer;
  }
  .sort-chip:hover {
    background: var(--accent-soft-strong);
  }
  .sort-chip-name {
    max-width: 180px;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .sort-chip-scope {
    font-weight: 400;
    white-space: nowrap;
  }
  .sort-server {
    height: 16px;
    padding: 0 6px;
    border: 1px solid var(--border);
    border-radius: 999px;
    font-size: var(--fs-xs);
    color: var(--text);
    background: transparent;
    cursor: pointer;
    white-space: nowrap;
  }
  .sort-server:hover {
    background: var(--surface-2);
  }
</style>
