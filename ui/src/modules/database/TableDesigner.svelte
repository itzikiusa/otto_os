<script lang="ts">
  // Workbench-style column designer for a SQL table. Edit column name / type /
  // NOT NULL / default, add or drop columns, then "Prepare ALTER" — which opens
  // the generated `ALTER TABLE …` in a query tab for the user to review and run
  // (never auto-applied). MySQL syntax (the platform's primary SQL engine).
  import Icon from '../../lib/components/Icon.svelte';
  import { database } from '../../lib/stores/database.svelte';
  import type { DbColumnDef } from '../../lib/api/types';

  interface Props {
    table: string;
    columns: DbColumnDef[];
    onclose: () => void;
  }
  let { table, columns, onclose }: Props = $props();

  interface Row {
    orig: string | null; // existing column name, or null for a new column
    name: string;
    type: string;
    notNull: boolean;
    def: string;
    drop: boolean;
  }

  function rowsFromColumns(cols: DbColumnDef[]): Row[] {
    return cols.map((c) => ({
      orig: c.name,
      name: c.name,
      type: c.data_type,
      notNull: !c.nullable,
      def: c.default ?? '',
      drop: false,
    }));
  }

  // Editable working copy of the table's columns. Re-seeded from the incoming
  // `columns` whenever the table being designed changes, so reopening the
  // designer on a different table never carries over the previous edits.
  let rows = $state<Row[]>([]);
  let seededFor = $state<string | null>(null);
  $effect(() => {
    if (seededFor !== table) {
      rows = rowsFromColumns(columns);
      seededFor = table;
    }
  });

  function addColumn(): void {
    rows = [
      ...rows,
      { orig: null, name: '', type: 'VARCHAR(255)', notNull: false, def: '', drop: false },
    ];
  }
  function quoteIdent(s: string): string {
    return '`' + s.replace(/`/g, '``') + '`';
  }
  function colDef(r: Row): string {
    let s = `${quoteIdent(r.name)} ${r.type}`;
    s += r.notNull ? ' NOT NULL' : ' NULL';
    if (r.def.trim() !== '') s += ` DEFAULT ${r.def.trim()}`;
    return s;
  }

  // Diff the edited rows against the originals → ALTER clauses.
  const sql = $derived.by(() => {
    const parts: string[] = [];
    for (const r of rows) {
      if (r.orig === null) {
        if (!r.drop && r.name.trim() && r.type.trim()) parts.push(`ADD COLUMN ${colDef(r)}`);
        continue;
      }
      if (r.drop) {
        parts.push(`DROP COLUMN ${quoteIdent(r.orig)}`);
        continue;
      }
      const orig = columns.find((c) => c.name === r.orig);
      if (!orig) continue;
      const changed =
        r.name !== r.orig ||
        r.type !== orig.data_type ||
        r.notNull === orig.nullable ||
        r.def !== (orig.default ?? '');
      if (changed && r.name.trim() && r.type.trim()) {
        parts.push(`CHANGE COLUMN ${quoteIdent(r.orig)} ${colDef(r)}`);
      }
    }
    return parts.length ? `ALTER TABLE ${quoteIdent(table)}\n  ${parts.join(',\n  ')};` : '';
  });

  function apply(): void {
    if (sql) void database.openInNewTab(sql);
    onclose();
  }

  // Escape closes the designer.
  $effect(() => {
    const onKey = (e: KeyboardEvent): void => {
      if (e.key === 'Escape') onclose();
    };
    window.addEventListener('keydown', onKey);
    return () => window.removeEventListener('keydown', onKey);
  });
</script>

<div class="td-backdrop">
  <div class="td-modal">
    <div class="td-head">
      <h3 class="mono"><Icon name="grid" size={14} /> Design {table}</h3>
      <button class="icon-btn" aria-label="Close" onclick={onclose}><Icon name="x" size={14} /></button>
    </div>

    <div class="td-cols">
      <div class="td-row td-hdr">
        <span>Column</span><span>Type</span><span class="ctr">NN</span><span>Default</span><span></span>
      </div>
      {#each rows as r, i (i)}
        <div class="td-row" class:dropped={r.drop}>
          <input class="mono" bind:value={r.name} placeholder="name" spellcheck="false" />
          <input class="mono" bind:value={r.type} placeholder="type" spellcheck="false" />
          <span class="ctr"><input type="checkbox" bind:checked={r.notNull} /></span>
          <input class="mono" bind:value={r.def} placeholder="NULL" spellcheck="false" />
          <button
            class="icon-btn"
            title={r.orig === null ? 'Remove' : r.drop ? 'Keep' : 'Drop column'}
            aria-label="Drop column"
            onclick={() => {
              if (r.orig === null) rows = rows.filter((_, j) => j !== i);
              else r.drop = !r.drop;
            }}
          ><Icon name="trash" size={12} /></button>
        </div>
      {/each}
      <button class="td-add" onclick={addColumn}><Icon name="plus" size={12} />Add column</button>
    </div>

    {#if sql}
      <pre class="td-preview mono">{sql}</pre>
    {:else}
      <div class="td-nochange dim">No changes yet.</div>
    {/if}

    <div class="td-foot">
      <span class="dim small">Opens the ALTER in a query tab to review &amp; run — nothing is applied automatically.</span>
      <span class="grow"></span>
      <button class="btn small" onclick={onclose}>Cancel</button>
      <button class="btn small primary" disabled={!sql} onclick={apply}>Prepare ALTER →</button>
    </div>
  </div>
</div>

<style>
  .td-backdrop {
    position: fixed;
    inset: 0;
    z-index: 60;
    background: rgba(0, 0, 0, 0.4);
    display: grid;
    place-items: center;
  }
  .td-modal {
    width: min(860px, 92vw);
    max-height: 86vh;
    overflow-y: auto;
    background: var(--surface);
    border: 1px solid var(--border);
    border-radius: var(--radius-m);
    box-shadow: 0 16px 48px rgba(0, 0, 0, 0.4);
    padding: 14px 16px;
    display: flex;
    flex-direction: column;
    gap: 12px;
  }
  .td-head {
    display: flex;
    align-items: center;
    justify-content: space-between;
  }
  .td-head h3 {
    display: flex;
    align-items: center;
    gap: 7px;
    font-size: 14px;
    margin: 0;
  }
  .td-cols {
    display: flex;
    flex-direction: column;
    gap: 4px;
  }
  .td-row {
    display: grid;
    grid-template-columns: 1.3fr 1.3fr 40px 1.2fr 28px;
    gap: 6px;
    align-items: center;
  }
  .td-hdr {
    font-size: 10.5px;
    font-weight: 700;
    text-transform: uppercase;
    letter-spacing: 0.04em;
    color: var(--text-dim);
  }
  .td-row.dropped input {
    text-decoration: line-through;
    opacity: 0.5;
  }
  .td-row input:not([type]) {
    height: 28px;
    padding: 0 8px;
    border: 1px solid var(--border);
    border-radius: var(--radius-s);
    background: var(--surface-2);
    color: var(--text);
    font-size: 12px;
    min-width: 0;
  }
  .ctr {
    display: grid;
    place-items: center;
  }
  .td-add {
    display: inline-flex;
    align-items: center;
    gap: 5px;
    align-self: flex-start;
    margin-top: 4px;
    height: 26px;
    padding: 0 9px;
    border: 1px dashed var(--border);
    border-radius: var(--radius-s);
    background: transparent;
    color: var(--text-dim);
    font-size: 11.5px;
    cursor: pointer;
  }
  .td-add:hover {
    color: var(--accent);
    border-color: color-mix(in srgb, var(--accent) 45%, transparent);
  }
  .td-preview {
    background: var(--bg);
    border: 1px solid var(--border);
    border-radius: var(--radius-s);
    padding: 10px;
    font-size: 12px;
    white-space: pre-wrap;
    margin: 0;
    color: var(--text);
  }
  .td-nochange {
    font-size: 12px;
    padding: 8px;
  }
  .td-foot {
    display: flex;
    align-items: center;
    gap: 8px;
  }
  .grow {
    flex: 1;
  }
</style>
