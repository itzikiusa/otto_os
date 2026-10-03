<script lang="ts">
  // CSV / TSV table: sticky header, row numbers, capped at CSV_ROW_CAP rows
  // (a note says how many were left out) so a large export never floods the DOM.
  import { parseCsv } from '../lib/csv';
  import { CSV_ROW_CAP, csvView } from './kinds';
  import PreviewError from './PreviewError.svelte';

  interface Props {
    content: string;
    delimiter: ',' | '\t';
  }
  let { content, delimiter }: Props = $props();

  const view = $derived.by(() => {
    try {
      return { ok: true as const, v: csvView(parseCsv(content, delimiter)) };
    } catch (e) {
      return { ok: false as const, error: e instanceof Error ? e.message : String(e) };
    }
  });
  const cols = $derived(view.ok ? Array.from({ length: view.v.width }, (_, i) => i) : []);
</script>

{#if !view.ok}
  <PreviewError title="Could not parse table" message={view.error} />
{:else if view.v.header.length === 0}
  <div class="state">Empty table — nothing to show yet.</div>
{:else}
  <div class="tbl-wrap">
    {#if view.v.total > view.v.rows.length}
      <p class="note" role="status">
        Showing the first {CSV_ROW_CAP.toLocaleString()} of {view.v.total.toLocaleString()} rows.
      </p>
    {/if}
    <!-- svelte-ignore a11y_no_noninteractive_tabindex (keyboard users scroll the wide table) -->
    <div class="scroller" tabindex="0" role="region" aria-label="Table preview">
      <table>
        <thead>
          <tr>
            <th class="num" scope="col">#</th>
            {#each cols as c (c)}
              <th scope="col">{view.v.header[c] ?? ''}</th>
            {/each}
          </tr>
        </thead>
        <tbody>
          {#each view.v.rows as r, i (i)}
            <tr>
              <td class="num">{i + 1}</td>
              {#each cols as c (c)}
                <td>{r[c] ?? ''}</td>
              {/each}
            </tr>
          {/each}
        </tbody>
      </table>
    </div>
  </div>
{/if}

<style>
  .tbl-wrap {
    display: flex;
    flex-direction: column;
    block-size: 100%;
    min-block-size: 0;
  }
  .note {
    margin: 0;
    padding: 6px 10px;
    font-size: var(--fs-s);
    color: var(--text-dim);
    border-bottom: 1px solid var(--border);
  }
  .scroller {
    flex: 1;
    min-block-size: 0;
    overflow: auto;
  }
  table {
    border-collapse: separate;
    border-spacing: 0;
    font-size: var(--fs-s);
    font-family: var(--font-mono);
  }
  th,
  td {
    padding: 4px 10px;
    border-bottom: 1px solid var(--border);
    border-inline-end: 1px solid var(--border);
    text-align: start;
    white-space: pre;
    max-inline-size: 420px;
    overflow: hidden;
    text-overflow: ellipsis;
  }
  th {
    position: sticky;
    inset-block-start: 0;
    background: var(--surface-2);
    font-weight: 600;
    z-index: 1;
  }
  .num {
    color: var(--text-dim);
    text-align: end;
    background: var(--surface);
  }
  th.num {
    background: var(--surface-2);
  }
  .state {
    padding: 12px;
    color: var(--text-dim);
    font-size: var(--fs-m);
  }
</style>
