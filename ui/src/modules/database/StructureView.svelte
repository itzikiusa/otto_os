<script lang="ts">
  // Object structure: a columns table (name/type/nullable/default/key), primary
  // key, indexes, foreign keys, and a collapsible DDL block. For Redis keys /
  // Mongo collections (no columns) it renders the `extra` JSON.
  import Icon from '../../lib/components/Icon.svelte';
  import EmptyState from '../../lib/components/EmptyState.svelte';
  import { database } from '../../lib/stores/database.svelte';
  import { toasts } from '../../lib/toast.svelte';

  const detail = $derived(database.objectDetail);
  let ddlOpen = $state(false);

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
      <button class="btn small ghost" onclick={explain}><Icon name="zap" size={11} />Explain</button>
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

    {#if detail.indexes.length > 0}
      <div class="block">
        <div class="block-title">Indexes <span class="count">{detail.indexes.length}</span></div>
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
