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

  // New indexes / foreign keys to ADD (this designer adds — it doesn't edit
  // existing index/FK metadata, which isn't passed in).
  interface IndexDef {
    name: string;
    cols: string; // comma-separated column names
    unique: boolean;
  }
  interface FkDef {
    name: string;
    cols: string; // comma-separated local columns
    refTable: string;
    refCols: string; // comma-separated referenced columns
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
  let indexes = $state<IndexDef[]>([]);
  let fks = $state<FkDef[]>([]);
  let seededFor = $state<string | null>(null);
  $effect(() => {
    if (seededFor !== table) {
      rows = rowsFromColumns(columns);
      indexes = [];
      fks = [];
      seededFor = table;
    }
  });

  function addColumn(): void {
    rows = [
      ...rows,
      { orig: null, name: '', type: 'VARCHAR(255)', notNull: false, def: '', drop: false },
    ];
  }
  function addIndex(): void {
    indexes = [...indexes, { name: '', cols: '', unique: false }];
  }
  function addFk(): void {
    fks = [...fks, { name: '', cols: '', refTable: '', refCols: '' }];
  }
  // Engine of the active connection — decides quoting + ALTER dialect. The
  // designer is only reachable for SQL engines (StructureView gates the button).
  const engine = $derived(database.capabilities?.engine ?? 'mysql');
  // ClickHouse has no foreign keys and no plain ADD INDEX-with-columns (its
  // secondary indexes are data-skipping and need a TYPE) — hide those sections.
  const isClickhouse = $derived(engine === 'clickhouse');

  function quoteIdent(s: string): string {
    return engine === 'postgres'
      ? '"' + s.replace(/"/g, '""') + '"'
      : '`' + s.replace(/`/g, '``') + '`';
  }
  /** Quote a comma-separated identifier list: `a, b ` → `` `a`, `b` ``. */
  function quoteCols(csv: string): string {
    return csv
      .split(',')
      .map((c) => c.trim())
      .filter(Boolean)
      .map(quoteIdent)
      .join(', ');
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
  const tableRef = $derived(
    objSchema ? `${quoteIdent(objSchema)}.${quoteIdent(table)}` : quoteIdent(table),
  );

  function colDef(r: Row, orig: DbColumnDef | null): string {
    let s = `${quoteIdent(r.name)} ${r.type}`;
    if (!isClickhouse) s += r.notNull ? ' NOT NULL' : ' NULL';
    if (r.def.trim() !== '') s += ` DEFAULT ${r.def.trim()}`;
    if (engine === 'mysql' && orig) {
      // CHANGE COLUMN replaces the WHOLE definition — without re-stating the
      // original attributes a rename silently drops AUTO_INCREMENT /
      // ON UPDATE CURRENT_TIMESTAMP and the comment. (DEFAULT_GENERATED is
      // information_schema bookkeeping, not valid DDL.)
      const extra = (orig.extra ?? '').replace(/DEFAULT_GENERATED/gi, '').trim();
      if (extra) s += ` ${extra}`;
      if (orig.comment) s += ` COMMENT '${orig.comment.replace(/'/g, "''")}'`;
    }
    return s;
  }

  interface RowDiff {
    r: Row;
    orig: DbColumnDef;
    renamed: boolean;
    typeChanged: boolean;
    nullChanged: boolean;
    defChanged: boolean;
  }
  /** Diff one edited row against its original column (null = unchanged). */
  function diffRow(r: Row): RowDiff | null {
    const orig = columns.find((c) => c.name === r.orig);
    if (!orig || !r.name.trim() || !r.type.trim()) return null;
    const d: RowDiff = {
      r,
      orig,
      renamed: r.name !== r.orig,
      typeChanged: r.type !== orig.data_type,
      nullChanged: r.notNull === orig.nullable,
      defChanged: r.def !== (orig.default ?? ''),
    };
    return d.renamed || d.typeChanged || d.nullChanged || d.defChanged ? d : null;
  }

  /** MySQL: one ALTER with comma-separated clauses (CHANGE COLUMN carries all). */
  function mysqlSql(): string {
    const parts: string[] = [];
    for (const r of rows) {
      if (r.orig === null) {
        if (!r.drop && r.name.trim() && r.type.trim())
          parts.push(`ADD COLUMN ${colDef(r, null)}`);
        continue;
      }
      if (r.drop) {
        parts.push(`DROP COLUMN ${quoteIdent(r.orig)}`);
        continue;
      }
      const d = diffRow(r);
      if (d) parts.push(`CHANGE COLUMN ${quoteIdent(r.orig!)} ${colDef(r, d.orig)}`);
    }
    // New indexes: ADD [UNIQUE] INDEX [name] (cols).
    for (const ix of indexes) {
      const cols = quoteCols(ix.cols);
      if (!cols) continue;
      const kw = ix.unique ? 'UNIQUE INDEX' : 'INDEX';
      const named = ix.name.trim() ? `${quoteIdent(ix.name.trim())} ` : '';
      parts.push(`ADD ${kw} ${named}(${cols})`);
    }
    // New foreign keys: ADD [CONSTRAINT name] FOREIGN KEY (cols) REFERENCES t (refcols).
    for (const fk of fks) {
      const cols = quoteCols(fk.cols);
      const refCols = quoteCols(fk.refCols);
      if (!cols || !fk.refTable.trim() || !refCols) continue;
      const named = fk.name.trim() ? `CONSTRAINT ${quoteIdent(fk.name.trim())} ` : '';
      parts.push(
        `ADD ${named}FOREIGN KEY (${cols}) REFERENCES ${quoteIdent(fk.refTable.trim())} (${refCols})`,
      );
    }
    return parts.length ? `ALTER TABLE ${tableRef}\n  ${parts.join(',\n  ')};` : '';
  }

  /** Postgres: standard SQL has no CHANGE COLUMN — each change is its own
   *  ALTER statement (rename first, then the rest address the new name). */
  function postgresSql(): string {
    const stmts: string[] = [];
    const alter = (clause: string): void => {
      stmts.push(`ALTER TABLE ${tableRef} ${clause};`);
    };
    for (const r of rows) {
      if (r.orig === null) {
        if (!r.drop && r.name.trim() && r.type.trim()) alter(`ADD COLUMN ${colDef(r, null)}`);
        continue;
      }
      if (r.drop) {
        alter(`DROP COLUMN ${quoteIdent(r.orig)}`);
        continue;
      }
      const d = diffRow(r);
      if (!d) continue;
      if (d.renamed) alter(`RENAME COLUMN ${quoteIdent(r.orig)} TO ${quoteIdent(r.name)}`);
      const col = quoteIdent(r.name);
      if (d.typeChanged) alter(`ALTER COLUMN ${col} TYPE ${r.type}`);
      if (d.nullChanged) alter(`ALTER COLUMN ${col} ${r.notNull ? 'SET' : 'DROP'} NOT NULL`);
      if (d.defChanged) {
        alter(
          r.def.trim() !== ''
            ? `ALTER COLUMN ${col} SET DEFAULT ${r.def.trim()}`
            : `ALTER COLUMN ${col} DROP DEFAULT`,
        );
      }
    }
    for (const ix of indexes) {
      const cols = quoteCols(ix.cols);
      if (!cols) continue;
      const named = ix.name.trim() ? `${quoteIdent(ix.name.trim())} ` : '';
      stmts.push(`CREATE ${ix.unique ? 'UNIQUE ' : ''}INDEX ${named}ON ${tableRef} (${cols});`);
    }
    for (const fk of fks) {
      const cols = quoteCols(fk.cols);
      const refCols = quoteCols(fk.refCols);
      if (!cols || !fk.refTable.trim() || !refCols) continue;
      const named = fk.name.trim() ? `CONSTRAINT ${quoteIdent(fk.name.trim())} ` : '';
      alter(
        `ADD ${named}FOREIGN KEY (${cols}) REFERENCES ${quoteIdent(fk.refTable.trim())} (${refCols})`,
      );
    }
    return stmts.join('\n');
  }

  /** ClickHouse: RENAME COLUMN + MODIFY COLUMN clauses; nullability is part of
   *  the type (`Nullable(T)`), FKs don't exist, indexes need a skipping TYPE. */
  function clickhouseSql(): string {
    const parts: string[] = [];
    for (const r of rows) {
      if (r.orig === null) {
        if (!r.drop && r.name.trim() && r.type.trim()) parts.push(`ADD COLUMN ${colDef(r, null)}`);
        continue;
      }
      if (r.drop) {
        parts.push(`DROP COLUMN ${quoteIdent(r.orig)}`);
        continue;
      }
      const d = diffRow(r);
      if (!d) continue;
      if (d.renamed) parts.push(`RENAME COLUMN ${quoteIdent(r.orig!)} TO ${quoteIdent(r.name)}`);
      if (d.typeChanged || d.defChanged) parts.push(`MODIFY COLUMN ${colDef(r, d.orig)}`);
    }
    return parts.length ? `ALTER TABLE ${tableRef}\n  ${parts.join(',\n  ')};` : '';
  }

  // Diff the edited rows against the originals → engine-correct ALTER SQL.
  const sql = $derived.by(() => {
    if (engine === 'postgres') return postgresSql();
    if (engine === 'clickhouse') return clickhouseSql();
    return mysqlSql();
  });

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
          <input dir="ltr" class="mono" bind:value={r.name} placeholder="name" spellcheck="false" />
          <input dir="ltr" class="mono" bind:value={r.type} placeholder="type" spellcheck="false" />
          <span class="ctr">
            <input
              type="checkbox"
              bind:checked={r.notNull}
              disabled={isClickhouse}
              title={isClickhouse ? 'ClickHouse nullability lives in the type — use Nullable(T)' : undefined}
            />
          </span>
          <input dir="ltr" class="mono" bind:value={r.def} placeholder="NULL" spellcheck="false" />
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
          <input dir="ltr" class="mono" bind:value={ix.name} placeholder="index name (optional)" spellcheck="false" />
          <input dir="ltr" class="mono" bind:value={ix.cols} placeholder="columns (comma-separated)" spellcheck="false" />
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
          <input dir="ltr" class="mono" bind:value={fk.cols} placeholder="column(s)" spellcheck="false" />
          <span class="td-fk-arrow">→</span>
          <input dir="ltr" class="mono" bind:value={fk.refTable} placeholder="referenced table" spellcheck="false" />
          <input dir="ltr" class="mono" bind:value={fk.refCols} placeholder="referenced column(s)" spellcheck="false" />
          <input dir="ltr" class="mono" bind:value={fk.name} placeholder="name (optional)" spellcheck="false" />
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
