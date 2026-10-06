<script lang="ts">
  // Workbench-style column designer for a SQL table. Edit column name / type /
  // NOT NULL / default, add or drop columns, then "Prepare ALTER" — which opens
  // the generated `ALTER TABLE …` in a query tab for the user to review and run
  // (never auto-applied). Generation is engine-aware: MySQL uses CHANGE COLUMN
  // clauses in one ALTER, Postgres a sequence of standard ALTER statements
  // (RENAME / ALTER COLUMN TYPE / SET-DROP NOT NULL / SET DEFAULT), ClickHouse
  // RENAME/MODIFY COLUMN (no FKs; nullability lives in the type).
  import Icon from '../../lib/components/Icon.svelte';
  import Modal from '../../lib/components/Modal.svelte';
  import { database } from '../../lib/stores/database.svelte';
  import type { DbColumnDef } from '../../lib/api/types';
  import { qualifiedName } from './sql-dialect';
  import {
    designerSql,
    newRow,
    rowsFromColumns,
    type DesignerFk,
    type DesignerIndex,
    type DesignerRow,
  } from './table-designer-sql';

  interface Props {
    table: string;
    columns: DbColumnDef[];
    onclose: () => void;
  }
  let { table, columns, onclose }: Props = $props();

  // Engine of the active connection — decides quoting + ALTER dialect. The
  // designer is only reachable for SQL engines (StructureView gates the button).
  const engine = $derived(database.capabilities?.engine ?? 'mysql');
  // ClickHouse has no foreign keys and no plain ADD INDEX-with-columns (its
  // secondary indexes are data-skipping and need a TYPE) — hide those sections.
  const isClickhouse = $derived(engine === 'clickhouse');

  // Editable working copy of the table's columns. Re-seeded from the incoming
  // `columns` whenever the table being designed changes, so reopening the
  // designer on a different table never carries over the previous edits.
  let rows = $state<DesignerRow[]>([]);
  let indexes = $state<DesignerIndex[]>([]);
  let fks = $state<DesignerFk[]>([]);
  let seededFor = $state<string | null>(null);
  $effect(() => {
    if (seededFor !== table) {
      rows = rowsFromColumns(engine, columns);
      indexes = [];
      fks = [];
      seededFor = table;
    }
  });

  function addColumn(): void {
    rows = [...rows, newRow()];
  }
  function addIndex(): void {
    indexes = [...indexes, { name: '', cols: '', unique: false }];
  }
  function addFk(): void {
    fks = [...fks, { name: '', cols: '', refTable: '', refCols: '' }];
  }
  // The prepared statement runs in a query tab against the ACTIVE db, which
  // need not be this table's db — qualify from the selected object's path
  // (`db:{db}/table:{t}` or `schema:{s}/table:{t}`).
  const objSchema = $derived.by(() => {
    const seg = (database.selectedObjectPath ?? '')
      .split('/')
      .find((s) => s.startsWith('db:') || s.startsWith('schema:'));
    return seg ? seg.slice(seg.indexOf(':') + 1) : null;
  });
  const tableRef = $derived(qualifiedName(engine, objSchema, table));

  // Diff the edited rows against the originals → engine-correct ALTER SQL
  // (the generators live in `table-designer-sql.ts`, unit-tested).
  const sql = $derived(designerSql({ engine, tableRef, columns, rows, indexes, fks }));

  function apply(): void {
    if (sql) void database.openInNewTab(sql);
    onclose();
  }
</script>

<!-- The shared Modal owns the backdrop, Esc / backdrop-click close, the focus
     trap + focus return, the Modal z-layer and ui.pushModal() (so the native
     browser pane hides under it). Focus lands on the first column field. -->
<Modal title="Design {table}" width={860} {onclose}>
  <div class="td-modal">
    <div class="td-cols">
      <div class="td-row td-hdr">
        <span>Column</span><span>Type</span><span class="ctr">NN</span><span>Default</span><span></span>
      </div>
      {#each rows as r, i (i)}
        <div class="td-row" class:dropped={r.drop}>
          <input dir="ltr" aria-label="Column name" class="mono" bind:value={r.name} placeholder="name" spellcheck="false" />
          <input dir="ltr" aria-label="Column type" class="mono" bind:value={r.type} placeholder="type" spellcheck="false" />
          <span class="ctr">
            <input
              type="checkbox"
              bind:checked={r.notNull}
              disabled={isClickhouse}
              title={isClickhouse ? 'ClickHouse nullability lives in the type — use Nullable(T)' : undefined}
            />
          </span>
          <input dir="ltr" aria-label="Column default" class="mono" bind:value={r.def} placeholder="NULL" spellcheck="false" />
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

    {#if isClickhouse}
      <!-- ClickHouse: secondary indexes are data-skipping (need a TYPE — use the
           Structure tab's index builder) and foreign keys don't exist. -->
      <div class="td-section">
        <div class="td-section-title">Indexes &amp; foreign keys</div>
        <div class="dim small">
          ClickHouse has no plain indexes or foreign keys — use the Structure tab’s
          index builder to add a data-skipping index.
        </div>
      </div>
    {:else}
    <!-- Indexes -->
    <div class="td-section">
      <div class="td-section-title">Indexes</div>
      {#each indexes as ix, i (i)}
        <div class="td-ix-row">
          <input dir="ltr" aria-label="Index name" class="mono" bind:value={ix.name} placeholder="index name (optional)" spellcheck="false" />
          <input dir="ltr" aria-label="Index columns" class="mono" bind:value={ix.cols} placeholder="columns (comma-separated)" spellcheck="false" />
          <label class="td-chk" title="Unique index"><input type="checkbox" bind:checked={ix.unique} />Unique</label>
          <button class="icon-btn" title="Remove index" aria-label="Remove index" onclick={() => (indexes = indexes.filter((_, j) => j !== i))}><Icon name="trash" size={12} /></button>
        </div>
      {/each}
      <button class="td-add" onclick={addIndex}><Icon name="plus" size={12} />Add index</button>
    </div>

    <!-- Foreign keys -->
    <div class="td-section">
      <div class="td-section-title">Foreign keys</div>
      {#each fks as fk, i (i)}
        <div class="td-fk-row">
          <input dir="ltr" aria-label="Foreign key columns" class="mono" bind:value={fk.cols} placeholder="column(s)" spellcheck="false" />
          <span class="td-fk-arrow">→</span>
          <input dir="ltr" aria-label="Referenced table" class="mono" bind:value={fk.refTable} placeholder="referenced table" spellcheck="false" />
          <input dir="ltr" aria-label="Referenced columns" class="mono" bind:value={fk.refCols} placeholder="referenced column(s)" spellcheck="false" />
          <input dir="ltr" aria-label="Foreign key name" class="mono" bind:value={fk.name} placeholder="name (optional)" spellcheck="false" />
          <button class="icon-btn" title="Remove foreign key" aria-label="Remove foreign key" onclick={() => (fks = fks.filter((_, j) => j !== i))}><Icon name="trash" size={12} /></button>
        </div>
      {/each}
      <button class="td-add" onclick={addFk}><Icon name="plus" size={12} />Add foreign key</button>
    </div>
    {/if}

    {#if sql}
      <pre class="td-preview mono">{sql}</pre>
    {:else}
      <div class="td-nochange dim">No changes yet.</div>
    {/if}

  </div>
  {#snippet footer()}
    <span class="dim small td-note">Opens the ALTER in a query tab to review &amp; run — nothing is applied automatically.</span>
    <span class="grow"></span>
    <button class="btn small" onclick={onclose}>Cancel</button>
    <button class="btn small primary" disabled={!sql} onclick={apply}>Prepare ALTER →</button>
  {/snippet}
</Modal>

<style>
  /* Body of the shared Modal (which scrolls it and caps it to the window). */
  .td-modal {
    display: flex;
    flex-direction: column;
    gap: 12px;
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
    font-size: var(--fs-xs);
    font-weight: 600;
    text-transform: uppercase;
    letter-spacing: .06em;
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
    font-size: var(--fs-s);
    min-width: 0;
  }
  .ctr {
    display: grid;
    place-items: center;
  }
  .td-section {
    display: flex;
    flex-direction: column;
    gap: 4px;
    border-top: 1px solid var(--border);
    padding-top: 10px;
  }
  .td-section-title {
    font-size: var(--fs-xs);
    font-weight: 600;
    text-transform: uppercase;
    letter-spacing: .06em;
    color: var(--text-dim);
  }
  .td-ix-row {
    display: grid;
    grid-template-columns: 1.3fr 1.6fr auto 28px;
    gap: 6px;
    align-items: center;
  }
  .td-fk-row {
    display: grid;
    grid-template-columns: 1fr auto 1.2fr 1.2fr 1fr 28px;
    gap: 6px;
    align-items: center;
  }
  .td-fk-arrow {
    color: var(--text-dim);
    text-align: center;
  }
  .td-ix-row input:not([type]),
  .td-fk-row input:not([type]) {
    height: 28px;
    padding: 0 8px;
    border: 1px solid var(--border);
    border-radius: var(--radius-s);
    background: var(--surface-2);
    color: var(--text);
    font-size: var(--fs-s);
    min-width: 0;
  }
  .td-chk {
    display: inline-flex;
    align-items: center;
    gap: 4px;
    font-size: var(--fs-s);
    color: var(--text-dim);
    white-space: nowrap;
  }
  .td-add {
    display: inline-flex;
    align-items: center;
    gap: 4px;
    align-self: flex-start;
    margin-top: 4px;
    height: 26px;
    padding: 0 8px;
    border: 1px dashed var(--border);
    border-radius: var(--radius-s);
    background: transparent;
    color: var(--text-dim);
    font-size: var(--fs-s);
    cursor: pointer;
  }
  .td-add:hover {
    color: var(--accent-text);
    border-color: var(--accent-line);
  }
  .td-preview {
    background: var(--bg);
    border: 1px solid var(--border);
    border-radius: var(--radius-s);
    padding: 10px;
    font-size: var(--fs-s);
    white-space: pre-wrap;
    margin: 0;
    color: var(--text);
  }
  .td-nochange {
    font-size: var(--fs-s);
    padding: 8px;
  }
  .td-note {
    align-self: center;
    min-width: 0;
  }
  .grow {
    flex: 1;
  }
</style>
