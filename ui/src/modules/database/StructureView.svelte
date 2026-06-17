<script lang="ts">
  // Object structure: a columns table (name/type/nullable/default/key), primary
  // key, indexes, foreign keys, and a collapsible DDL block. For Redis keys /
  // Mongo collections (no columns) it renders the `extra` JSON.
  import Icon from '../../lib/components/Icon.svelte';
  import EmptyState from '../../lib/components/EmptyState.svelte';
  import TableDesigner from './TableDesigner.svelte';
  import { database } from '../../lib/stores/database.svelte';
  import { toasts } from '../../lib/toast.svelte';

  const detail = $derived(database.objectDetail);
  let ddlOpen = $state(false);
  let designerOpen = $state(false);

  function prettyExtra(extra: unknown): string {
    try {
      return JSON.stringify(extra, null, 2);
    } catch {
      return String(extra);
    }
  }

  async function copyDdl(): Promise<void> {
    if (!detail?.ddl) return;
    try {
      await navigator.clipboard.writeText(detail.ddl);
      toasts.success('Copied DDL');
    } catch {
      toasts.error('Copy failed');
    }
  }

  function explain(): void {
    if (!detail) return;
    const content = detail.ddl
      ? `DDL for ${detail.name}:\n\n${detail.ddl}`
      : `Object ${detail.name} (${detail.kind})\n\n${prettyExtra(detail.extra)}`;
    void database.explainWithAgent(
      content,
      `Explain the structure of ${detail.name} and how it is used.`,
      `Explain ${detail.name}`,
    );
  }

  // ── Index builder (per-engine) ──────────────────────────────────────────────
  const isSql = $derived(database.capabilities?.sql === true);
  const canIndex = $derived(
    !!detail && (isSql ? detail.kind === 'table' : detail.kind === 'collection'),
  );
  const indexFields = $derived.by(() => {
    if (!detail) return [] as string[];
    if (isSql) return detail.columns.map((c) => c.name);
    const sampled = (detail.extra as Record<string, unknown> | null)?.sampled_fields as
      | Record<string, unknown>
      | undefined;
    return ['_id', ...Object.keys(sampled ?? {}).filter((f) => f !== '_id')];
  });
  let idxOpen = $state(false);
  let idxCols = $state<string[]>([]);
  let idxUnique = $state(false);

  function toggleIdxCol(c: string): void {
    idxCols = idxCols.includes(c) ? idxCols.filter((x) => x !== c) : [...idxCols, c];
  }
  // Prepare a CREATE INDEX / createIndex statement and open it in a query tab
  // for the user to review and run (never auto-applied).
  function buildIndex(): void {
    if (!detail || idxCols.length === 0) return;
    const name = `idx_${detail.name}_${idxCols.join('_')}`.replace(/[^A-Za-z0-9_]/g, '_');
    const stmt = isSql
      ? `CREATE ${idxUnique ? 'UNIQUE ' : ''}INDEX ${name} ON ${detail.name} (${idxCols.join(', ')});`
      : `db.${detail.name}.createIndex({ ${idxCols.map((c) => `"${c}": 1`).join(', ')} }${idxUnique ? ', { "unique": true }' : ''})`;
    void database.openInNewTab(stmt);
    idxOpen = false;
    idxCols = [];
    idxUnique = false;
  }

  // ── Stats (Mongo collStats) ─────────────────────────────────────────────────
  const SIZE_KEYS = new Set(['size', 'storageSize', 'avgObjSize', 'totalIndexSize', 'totalSize']);
  function fmtBytes(n: number): string {
    if (n < 1024) return `${n} B`;
    const u = ['KB', 'MB', 'GB', 'TB'];
    let v = n / 1024;
    let i = 0;
    while (v >= 1024 && i < u.length - 1) {
      v /= 1024;
      i++;
    }
    return `${v.toFixed(1)} ${u[i]}`;
  }
  const mongoStats = $derived.by(() => {
    const s = (detail?.extra as Record<string, unknown> | null)?.stats as
      | Record<string, unknown>
      | undefined;
    if (!s) return null;
    return Object.entries(s).map(
      ([k, v]) =>
        [k, SIZE_KEYS.has(k) && typeof v === 'number' ? fmtBytes(v) : String(v)] as [string, string],
    );
  });
</script>

<div class="structure">
  {#if database.objectLoading}
    <div class="loading"><Icon name="refresh" size={16} /><span>Loading structure…</span></div>
  {:else if !detail}
    <EmptyState icon="box" title="No object selected" body="Pick a table, view, collection or key from the schema tree to inspect its structure." />
  {:else}
    <div class="st-head">
      <div class="st-title">
        <Icon name="grid" size={15} />
        <h2 class="mono">{detail.name}</h2>
        <span class="kind-chip">{detail.kind}</span>
        {#if detail.row_count != null}
          <span class="rowcount">{detail.row_count.toLocaleString()} rows</span>
        {/if}
      </div>
      <div class="st-head-actions">
        {#if isSql && detail.kind === 'table'}
          <button class="btn small ghost" onclick={() => (designerOpen = true)} title="Edit columns → generates ALTER TABLE for review">
            <Icon name="edit" size={11} />Design
          </button>
        {/if}
        <button class="btn small ghost" onclick={explain}><Icon name="zap" size={11} />Explain</button>
      </div>
    </div>

    {#if detail.columns.length > 0}
      <div class="block">
        <div class="block-title">Columns <span class="count">{detail.columns.length}</span></div>
        <div class="tbl-wrap">
          <table class="tbl mono">
            <thead>
              <tr><th>Name</th><th>Type</th><th>Null</th><th>Key</th><th>Default</th><th>Extra</th></tr>
            </thead>
            <tbody>
              {#each detail.columns as c, i (i)}
                <tr>
                  <td class="cn">
                    {c.name}
                    {#if detail.primary_key.includes(c.name)}<span class="pk" title="Primary key">PK</span>{/if}
                  </td>
                  <td class="ty">{c.data_type}</td>
                  <td class="nullable">{c.nullable ? 'YES' : 'NO'}</td>
                  <td>{c.key ?? ''}</td>
                  <td class="dim">{c.default ?? ''}</td>
                  <td class="dim">{c.extra ?? ''}</td>
                </tr>
                {#if c.comment}
                  <tr class="comment-row"><td></td><td colspan="5" class="comment">{c.comment}</td></tr>
                {/if}
              {/each}
            </tbody>
          </table>
        </div>
      </div>
    {/if}

    {#if detail.primary_key.length > 0}
      <div class="block">
        <div class="block-title">Primary key</div>
        <div class="chips">
          {#each detail.primary_key as col (col)}<span class="key-chip mono">{col}</span>{/each}
        </div>
      </div>
    {/if}

    {#if detail.indexes.length > 0 || canIndex}
      <div class="block">
        <div class="block-title">
          Indexes <span class="count">{detail.indexes.length}</span>
          <span class="grow"></span>
          {#if canIndex}
            <button class="mini-btn" onclick={() => (idxOpen = !idxOpen)}>
              <Icon name="plus" size={11} />New index
            </button>
          {/if}
        </div>
        {#if detail.indexes.length > 0}
          <ul class="idx-list">
            {#each detail.indexes as idx, i (i)}
              <li class="idx">
                <Icon name="key" size={11} />
                <span class="idx-name mono">{idx.name}</span>
                {#if idx.unique}<span class="tag unique">unique</span>{/if}
                {#if idx.method}<span class="tag">{idx.method}</span>{/if}
                <span class="idx-cols mono">({idx.columns.join(', ')})</span>
              </li>
            {/each}
          </ul>
        {/if}
        {#if idxOpen}
          <div class="idx-builder">
            <div class="ib-fields">
              {#each indexFields as f (f)}
                <button
                  class="ib-chip mono"
                  class:on={idxCols.includes(f)}
                  onclick={() => toggleIdxCol(f)}
                >{idxCols.includes(f) ? `${idxCols.indexOf(f) + 1}· ` : ''}{f}</button>
              {/each}
            </div>
            <label class="ib-unique"><input type="checkbox" bind:checked={idxUnique} /> Unique</label>
            <div class="ib-actions">
              <button class="btn small" onclick={() => (idxOpen = false)}>Cancel</button>
              <button class="btn small primary" disabled={idxCols.length === 0} onclick={buildIndex}>
                Prepare {isSql ? 'CREATE INDEX' : 'createIndex'} →
              </button>
            </div>
            <div class="ib-hint dim">Opens the statement in a query tab for you to review and run.</div>
          </div>
        {/if}
      </div>
    {/if}

    {#if mongoStats}
      <div class="block">
        <div class="block-title">Stats</div>
        <div class="stats-grid">
          {#each mongoStats as [k, v] (k)}
            <div class="stat"><span class="sk mono">{k}</span><span class="sv mono">{v}</span></div>
          {/each}
        </div>
      </div>
    {/if}

    {#if detail.foreign_keys.length > 0}
      <div class="block">
        <div class="block-title">Foreign keys <span class="count">{detail.foreign_keys.length}</span></div>
        <ul class="fk-list">
          {#each detail.foreign_keys as fk, i (i)}
            <li class="fk">
              <span class="fk-name mono">{fk.name}</span>
              <span class="fk-map mono">
                ({fk.columns.join(', ')})
                <Icon name="arrowDown" size={10} />
                {fk.ref_schema ? `${fk.ref_schema}.` : ''}{fk.ref_table}({fk.ref_columns.join(', ')})
              </span>
            </li>
          {/each}
        </ul>
      </div>
    {/if}

    {#if detail.extra != null && detail.columns.length === 0}
      <div class="block">
        <div class="block-title">Details</div>
        <pre class="extra mono">{prettyExtra(detail.extra)}</pre>
      </div>
    {/if}

    {#if detail.ddl}
      <div class="block">
        <div class="ddl-head">
          <button class="block-title toggle" onclick={() => (ddlOpen = !ddlOpen)}>
            <Icon name={ddlOpen ? 'chevronDown' : 'chevronRight'} size={11} />
            DDL
          </button>
          <span class="grow"></span>
          {#if ddlOpen}
            <button class="copy-ddl" onclick={copyDdl}>
              <Icon name="file" size={11} />Copy
            </button>
          {/if}
        </div>
        {#if ddlOpen}
          <pre class="ddl mono">{detail.ddl}</pre>
        {/if}
      </div>
    {/if}
  {/if}
</div>

{#if designerOpen && detail && detail.kind === 'table'}
  <TableDesigner
    table={detail.name}
    columns={detail.columns}
    onclose={() => (designerOpen = false)}
  />
{/if}

<style>
  .structure {
    height: 100%;
    overflow-y: auto;
    padding: 4px 2px 24px;
  }
  .loading {
    display: flex;
    align-items: center;
    gap: 8px;
    padding: 24px;
    color: var(--text-dim);
    font-size: 12.5px;
  }
  .st-head-actions {
    display: flex;
    gap: 6px;
    flex-shrink: 0;
  }
  .st-head {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 12px;
    padding-bottom: 8px;
    border-bottom: 1px solid var(--border);
    margin-bottom: 14px;
  }
  .st-title {
    display: flex;
    align-items: center;
    gap: 9px;
    color: var(--accent);
  }
  .st-title h2 {
    margin: 0;
    font-size: 15px;
    font-weight: 600;
    color: var(--text);
  }
  .kind-chip {
    font-size: 10px;
    text-transform: uppercase;
    letter-spacing: 0.04em;
    color: var(--text-dim);
    background: var(--surface-2);
    padding: 1px 7px;
    border-radius: 999px;
  }
  .rowcount {
    font-size: 11px;
    color: var(--text-dim);
  }
  .block {
    margin-bottom: 18px;
  }
  .block-title {
    display: flex;
    align-items: center;
    gap: 6px;
    font-size: 11px;
    font-weight: 700;
    text-transform: uppercase;
    letter-spacing: 0.05em;
    color: var(--text-dim);
    margin-bottom: 8px;
  }
  .grow {
    flex: 1;
  }
  .mini-btn {
    display: inline-flex;
    align-items: center;
    gap: 4px;
    height: 22px;
    padding: 0 8px;
    border: 1px solid var(--border);
    border-radius: var(--radius-s);
    background: var(--surface-2);
    color: var(--text);
    font-size: 11px;
    cursor: pointer;
    text-transform: none;
    letter-spacing: 0;
  }
  .mini-btn:hover {
    border-color: color-mix(in srgb, var(--accent) 45%, transparent);
    color: var(--accent);
  }
  .idx-builder {
    margin-top: 8px;
    padding: 10px;
    border: 1px solid var(--border);
    border-radius: var(--radius-m);
    background: var(--surface-2);
    display: flex;
    flex-direction: column;
    gap: 8px;
  }
  .ib-fields {
    display: flex;
    flex-wrap: wrap;
    gap: 5px;
  }
  .ib-chip {
    padding: 3px 8px;
    border: 1px solid var(--border);
    border-radius: 999px;
    background: var(--surface);
    color: var(--text);
    font-size: 11.5px;
    cursor: pointer;
  }
  .ib-chip.on {
    border-color: var(--accent);
    background: color-mix(in srgb, var(--accent) 16%, transparent);
    color: var(--accent);
  }
  .ib-unique {
    display: flex;
    align-items: center;
    gap: 6px;
    font-size: 12px;
    color: var(--text);
  }
  .ib-actions {
    display: flex;
    gap: 6px;
  }
  .ib-hint {
    font-size: 11px;
  }
  .stats-grid {
    display: grid;
    grid-template-columns: repeat(auto-fill, minmax(150px, 1fr));
    gap: 6px;
  }
  .stat {
    display: flex;
    justify-content: space-between;
    gap: 8px;
    padding: 5px 8px;
    border: 1px solid var(--border);
    border-radius: var(--radius-s);
    background: var(--surface-2);
    font-size: 11.5px;
  }
  .stat .sk {
    color: var(--text-dim);
  }
  .ddl-head {
    display: flex;
    align-items: center;
    gap: 6px;
  }
  .block-title.toggle {
    border: none;
    background: transparent;
    cursor: pointer;
    padding: 4px 0;
    text-align: left;
    color: var(--text-dim);
  }
  .copy-ddl {
    display: inline-flex;
    align-items: center;
    gap: 4px;
    font-size: 10.5px;
    color: var(--text-dim);
    cursor: pointer;
    border: none;
    background: transparent;
    font-weight: 500;
  }
  .copy-ddl:hover {
    color: var(--accent);
  }
  .count {
    color: var(--text-dim);
    font-weight: 500;
  }
  .tbl-wrap {
    border: 1px solid var(--border);
    border-radius: var(--radius-s);
    overflow: auto;
  }
  .tbl {
    width: 100%;
    border-collapse: collapse;
    user-select: text;
  }
  .tbl th {
    text-align: left;
    padding: 6px 10px;
    background: var(--surface-2);
    border-bottom: 1px solid var(--border);
    font-size: 10.5px;
    text-transform: uppercase;
    letter-spacing: 0.03em;
    color: var(--text-dim);
    white-space: nowrap;
  }
  .tbl td {
    padding: 5px 10px;
    border-bottom: 1px solid color-mix(in srgb, var(--border) 60%, transparent);
    font-size: 11.5px;
    vertical-align: top;
  }
  .tbl tbody tr:hover td {
    background: color-mix(in srgb, var(--accent) 6%, transparent);
  }
  .cn {
    font-weight: 600;
    color: var(--text);
    white-space: nowrap;
  }
  .ty {
    color: #0e8a8a;
  }
  :global(html[data-scheme='dark']) .ty {
    color: #56c8d8;
  }
  .nullable {
    color: var(--text-dim);
    font-size: 10.5px;
  }
  .pk {
    margin-left: 6px;
    font-size: 9px;
    font-weight: 700;
    color: var(--accent);
    background: color-mix(in srgb, var(--accent) 16%, transparent);
    padding: 0 4px;
    border-radius: 3px;
    vertical-align: middle;
  }
  .comment-row td {
    border-bottom: 1px solid color-mix(in srgb, var(--border) 60%, transparent);
  }
  .comment {
    color: var(--text-dim);
    font-style: italic;
    font-size: 11px;
  }
  .chips,
  .idx-list,
  .fk-list {
    display: flex;
    flex-direction: column;
    gap: 5px;
    margin: 0;
    padding: 0;
    list-style: none;
  }
  .chips {
    flex-direction: row;
    flex-wrap: wrap;
  }
  .key-chip {
    font-size: 11.5px;
    background: var(--surface-2);
    border: 1px solid var(--border);
    padding: 2px 9px;
    border-radius: 999px;
    color: var(--text);
  }
  .idx,
  .fk {
    display: flex;
    align-items: center;
    gap: 8px;
    padding: 5px 10px;
    background: var(--surface-2);
    border: 1px solid var(--border);
    border-radius: var(--radius-s);
    font-size: 11.5px;
  }
  .idx-name,
  .fk-name {
    font-weight: 600;
    color: var(--text);
  }
  .idx-cols,
  .fk-map {
    color: var(--text-dim);
    display: inline-flex;
    align-items: center;
    gap: 4px;
  }
  .tag {
    font-size: 9.5px;
    text-transform: uppercase;
    color: var(--text-dim);
    background: color-mix(in srgb, var(--text-dim) 16%, transparent);
    padding: 0 5px;
    border-radius: 999px;
  }
  .tag.unique {
    color: var(--accent);
    background: color-mix(in srgb, var(--accent) 16%, transparent);
  }
  .ddl,
  .extra {
    margin: 0;
    background: var(--surface-2);
    border: 1px solid var(--border);
    border-radius: var(--radius-s);
    padding: 12px;
    font-size: 11.5px;
    line-height: 1.5;
    overflow: auto;
    max-height: 360px;
    user-select: text;
    white-space: pre;
  }
  .extra {
    white-space: pre-wrap;
    word-break: break-word;
  }
  .dim {
    color: var(--text-dim);
  }
  .grow {
    flex: 1;
  }
</style>
