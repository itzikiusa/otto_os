<script lang="ts">
  // CSV / TSV as a read-only table (sticky header, first 120 rows) and JSON
  // Lines as one JSON tree per line — the rendered half of Split for tabular
  // files. The IDE's editable grid (IdeCsvView) stays its full Preview.
  import JsonTree from '../JsonTree.svelte';
  import { parseDelimitedTable } from '../../render/previewText';
  import PreviewNotice from './PreviewNotice.svelte';

  interface Props {
    content: string;
    kind: 'csv' | 'tsv' | 'jsonl';
  }
  let { content, kind }: Props = $props();

  const JSONL_MAX = 200;
  const table = $derived(kind === 'jsonl' ? null : parseDelimitedTable(content, kind === 'tsv' ? '\t' : ','));
  const jsonl = $derived.by(() => {
    if (kind !== 'jsonl') return null;
    const values: unknown[] = [];
    const lines = content.split(/\r?\n/);
    for (let i = 0; i < lines.length && values.length < JSONL_MAX; i++) {
      if (!lines[i].trim()) continue;
      try {
        values.push(JSON.parse(lines[i]));
      } catch (e) {
        return { values, error: { line: i + 1, message: e instanceof SyntaxError ? e.message : 'Invalid JSON' } };
      }
    }
    return { values, error: null };
  });
</script>

{#if jsonl}
  {#if jsonl.error}
    <PreviewNotice title="Invalid JSON line" message={jsonl.error.message} line={jsonl.error.line} />
  {/if}
  <div class="fp-jsonl">
    {#each jsonl.values as v, i (i)}
      <div class="fp-jsonl-row"><span class="fp-jsonl-n">{i + 1}</span><JsonTree value={v} /></div>
    {/each}
  </div>
{:else if table && table.headers.length === 0}
  <p class="fp-table-empty">Empty table — nothing to show yet.</p>
{:else if table}
  <div class="fp-table-wrap">
    <table class="fp-table">
      <thead>
        <tr>
          {#each table.headers as h, idx (idx)}
            <th>{h}</th>
          {/each}
        </tr>
      </thead>
      <tbody>
        {#each table.rows as row, rIdx (rIdx)}
          <tr>
            {#each table.headers as _, cIdx (cIdx)}
              <td>{row[cIdx] ?? ''}</td>
            {/each}
          </tr>
        {/each}
      </tbody>
    </table>
  </div>
{/if}

<style>
  .fp-table-wrap {
    overflow: auto;
    block-size: 100%;
  }
  .fp-table {
    inline-size: 100%;
    border-collapse: collapse;
    font-size: var(--fs-xs);
  }
  .fp-table th,
  .fp-table td {
    padding: 6px 8px;
    border-block-end: 1px solid var(--border);
    text-align: start;
    white-space: nowrap;
  }
  .fp-table th {
    position: sticky;
    inset-block-start: 0;
    background: var(--surface-2);
    font-weight: 600;
    color: var(--text);
  }
  .fp-table-empty {
    margin: 0;
    padding: 12px;
    color: var(--text-dim);
    font-size: var(--fs-m);
  }
  .fp-jsonl {
    padding: 8px 12px;
    font-family: var(--font-mono);
    font-size: var(--fs-s);
  }
  .fp-jsonl-row {
    display: flex;
    gap: 8px;
    padding-block: 2px;
    border-block-end: 1px solid var(--border);
    overflow-x: auto;
  }
  .fp-jsonl-n {
    color: var(--text-dim);
    flex-shrink: 0;
  }
</style>
