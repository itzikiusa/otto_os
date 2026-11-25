<script lang="ts">
  // One watched table in the DB Watch panel: a header with the counts and an
  // honest window note, then the changed rows — added / removed tinted, a
  // changed row as its key plus ONLY the changed cells (before → after), a
  // click opens the full record one field per line. 20 rows, then "N more".
  import Icon from '../../../lib/components/Icon.svelte';
  import LoadState from '../../../lib/components/LoadState.svelte';
  import { plural } from '../../../lib/plural';
  import type { IdeDbRowDiff, IdeDbWatchItemDiff, IdeDbWatchItemInfo, IdeDbWatchSnapshotItem } from '../../../lib/api/types/ide-db-watch';
  import {
    cellChanges,
    currentAsDiffs,
    filterText,
    recordFields,
    rowKeys,
    rowPreview,
    ROWS_SHOWN,
  } from './ideDbWatchModel';

  interface Props {
    item: IdeDbWatchItemDiff;
    info: IdeDbWatchItemInfo | undefined;
    /** "Show current rows" result for this table, when asked. */
    current?: IdeDbWatchSnapshotItem | null;
    /** Where the code touches it (the name jumps there). */
    codeRef?: { file: string; line: number } | null;
    /** Only the baseline exists: no counts yet. */
    waiting?: boolean;
    onmenu: (e: MouseEvent) => void;
    onretry: () => void;
    onjump?: () => void;
  }
  let { item, info, current = null, codeRef = null, waiting = false, onmenu, onretry, onjump }: Props = $props();

  let expanded = $state<string | null>(null);
  let showAll = $state(false);

  const keyCol = $derived(info?.primary_key?.join(', ') || 'key');
  const changed = $derived(item.added_count + item.modified_count + item.removed_count);
  const rows = $derived(showAll ? item.diffs : item.diffs.slice(0, ROWS_SHOWN));
  const hiddenCount = $derived(item.diffs.length - rows.length + item.more_count);
  const currentRows = $derived<IdeDbRowDiff[]>(current ? currentAsDiffs(current, info?.primary_key ?? []) : []);

  function sign(kind: IdeDbRowDiff['kind']): string {
    return kind === 'added' ? '+' : kind === 'removed' ? '−' : '~';
  }
  function kindWord(kind: IdeDbRowDiff['kind']): string {
    return kind === 'added' ? 'Added' : kind === 'removed' ? 'Removed' : 'Changed';
  }
</script>

{#snippet rowList(list: IdeDbRowDiff[], plain: boolean)}
  {@const keys = rowKeys(list)}
  <ul class="dbt-rows" role="list">
    {#each list as d, i (keys[i])}
      {@const open = expanded === keys[i]}
      <li class="dbt-li">
        <button
          type="button"
          class="dbt-row is-{plain ? 'plain' : d.kind}"
          aria-expanded={open}
          title={plain ? `${keyCol} = ${d.key}` : `${kindWord(d.kind)} row ${keyCol} = ${d.key}`}
          onclick={() => (expanded = open ? null : keys[i])}
          data-testid="ide-dbw-diff-row"
          data-kind={d.kind}
        >
          {#if !plain}<span class="dbt-sign" aria-hidden="true">{sign(d.kind)}</span>{/if}
          <span class="dbt-key mono" dir="ltr">{d.key}</span>
          <span class="dbt-cells mono" dir="ltr">
            {#if d.kind === 'modified'}
              {#each cellChanges(d.changes) as c (c.path)}
                <span class="dbt-cell"><span class="dbt-col">{c.path}</span>
                  {#if c.hidden}<span class="dbt-hidden">changed · hidden</span>
                  {:else}<span class="dbt-before">{c.before}</span><span class="dbt-arrow" aria-label="changed to">→</span><span class="dbt-after">{c.after}</span>{/if}
                </span>
              {/each}
            {:else if d.masked}
              <span class="dbt-hidden">hidden</span>
            {:else}
              {rowPreview(current && plain ? current.columns : item.columns, d.kind === 'removed' ? (d.previous_row ?? d.row) : d.row)}
            {/if}
          </span>
          <Icon name={open ? 'chevronUp' : 'chevronDown'} size={12} />
        </button>
        {#if open}
          <dl class="dbt-record mono" dir="ltr" data-testid="ide-dbw-record">
            {#each recordFields(current && plain ? current.columns : item.columns, d) as f (f.path)}
              <div class="dbt-field" class:is-changed={f.changed}>
                <dt>{f.path}</dt>
                <dd class:is-null={f.isNull}>
                  {#if f.before !== undefined}<span class="dbt-before">{f.before}</span><span class="dbt-arrow" aria-label="changed to">→</span>{/if}{f.value}
                </dd>
              </div>
            {/each}
          </dl>
        {/if}
      </li>
    {/each}
  </ul>
{/snippet}

<section class="dbt" aria-label="{item.object} changes" data-testid="ide-dbw-table" data-object={item.object}>
  <header class="dbt-head">
    {#if onjump && codeRef}
      <button type="button" class="dbt-name mono" dir="ltr" title="Open {codeRef.file}:{codeRef.line}" onclick={onjump}>{item.object}</button>
    {:else}
      <span class="dbt-name mono" dir="ltr">{item.object}</span>
    {/if}
    {#if info?.filter}<span class="dbt-filter mono" dir="ltr" title="Only rows where {filterText(info.filter)}">{filterText(info.filter)}</span>{/if}
    <span class="dbt-counts" data-testid="ide-dbw-table-counts">
      {#if item.error}
        <span class="dbt-err-word">error</span>
      {:else if waiting}
        <span class="dbt-none">baseline</span>
      {:else if changed === 0}
        <span class="dbt-none">no changes</span>
      {:else}
        {#if item.added_count}<span class="dbt-add">+{item.added_count} added</span>{/if}
        {#if item.modified_count}<span class="dbt-mod">~{item.modified_count} changed</span>{/if}
        {#if item.removed_count}<span class="dbt-del">−{item.removed_count} removed</span>{/if}
      {/if}
    </span>
    <span class="dbt-window" class:is-full={item.window_full} title={item.window_note}>{item.window_note}</span>
    <button type="button" class="icon-btn dbt-more" aria-label="{item.object} options" title="{item.object}: filter, current rows, copy query, remove" aria-haspopup="menu" onclick={onmenu}>
      <Icon name="more" size={13} />
    </button>
  </header>

  {#if item.error}
    <LoadState what={item.object} errorTitle="{item.object}: {item.error}" error={item.error} variant="compact" {onretry} />
  {:else if current}
    {#if currentRows.length === 0}
      <p class="dbt-quiet">No rows.</p>
    {:else}
      {@render rowList(showAll ? currentRows : currentRows.slice(0, ROWS_SHOWN), true)}
      {#if !showAll && currentRows.length > ROWS_SHOWN}
        <button type="button" class="btn small ghost dbt-show" onclick={() => (showAll = true)}>{plural(currentRows.length - ROWS_SHOWN, 'more row')} · Show</button>
      {/if}
    {/if}
  {:else if changed > 0}
    {@render rowList(rows, false)}
    {#if hiddenCount > 0}
      {#if !showAll && item.diffs.length > ROWS_SHOWN}
        <button type="button" class="btn small ghost dbt-show" onclick={() => (showAll = true)} data-testid="ide-dbw-show-more">{plural(hiddenCount, 'more row')} · Show</button>
      {:else}
        <p class="dbt-quiet">{plural(item.more_count, 'more changed row')} not loaded (the daemon keeps 150 per table).</p>
      {/if}
    {/if}
  {/if}
</section>

<style>
  .dbt { border-block-end: 1px solid var(--separator); }
  .dbt-head {
    position: sticky; inset-block-start: 0; z-index: var(--z-sticky);
    display: flex; align-items: center; gap: var(--sp-3);
    block-size: 26px; padding-inline: var(--sp-4) var(--sp-2);
    background: var(--surface-2); font-size: var(--fs-xs); min-inline-size: 0;
  }
  .dbt-name { font-weight: 600; color: var(--text); border: 0; background: none; padding: 0; white-space: nowrap; font-size: var(--fs-xs); }
  button.dbt-name { cursor: pointer; }
  button.dbt-name:hover { color: var(--accent-text); text-decoration: underline; }
  .dbt-filter { color: var(--text-dim); white-space: nowrap; overflow: hidden; text-overflow: ellipsis; max-inline-size: 30%; }
  .dbt-counts { display: inline-flex; gap: var(--sp-3); white-space: nowrap; }
  .dbt-add { color: var(--success); }
  .dbt-mod { color: var(--warning); }
  .dbt-del, .dbt-err-word { color: var(--danger); }
  .dbt-none { color: var(--text-dim); }
  .dbt-window { margin-inline-start: auto; color: var(--text-dim); white-space: nowrap; overflow: hidden; text-overflow: ellipsis; min-inline-size: 0; }
  .dbt-window.is-full { color: var(--warning); }
  .dbt-more { flex-shrink: 0; block-size: 22px; inline-size: 22px; }
  .dbt-rows { list-style: none; margin: 0; padding: 0; }
  .dbt-row {
    display: grid; grid-template-columns: 12px minmax(3ch, max-content) minmax(0, 1fr) 12px; align-items: center; gap: var(--sp-3);
    inline-size: 100%; block-size: 22px; padding-inline: var(--sp-4) var(--sp-3);
    border: 0; border-block-end: 1px solid var(--separator); background: transparent; color: var(--text);
    font-size: var(--fs-xs); text-align: start; cursor: pointer;
  }
  .dbt-row.is-plain { grid-template-columns: minmax(3ch, max-content) minmax(0, 1fr) 12px; }
  .dbt-row:hover { background: var(--hover); }
  .dbt-row.is-added { background: var(--success-soft); }
  .dbt-row.is-removed { background: var(--danger-soft); }
  .dbt-row.is-removed .dbt-cells { color: var(--text-dim); text-decoration: line-through; }
  .dbt-row.is-added:hover, .dbt-row.is-removed:hover { filter: brightness(0.97); }
  .dbt-row :global(svg) { color: var(--text-dim); }
  .dbt-sign { font-weight: 600; text-align: center; }
  .is-added .dbt-sign { color: var(--success); }
  .is-removed .dbt-sign { color: var(--danger); }
  .is-modified .dbt-sign { color: var(--warning); }
  .dbt-key { color: var(--text-dim); white-space: nowrap; max-inline-size: 22ch; overflow: hidden; text-overflow: ellipsis; }
  .dbt-cells { white-space: nowrap; overflow: hidden; text-overflow: ellipsis; min-inline-size: 0; }
  .dbt-cell { margin-inline-end: var(--sp-5); }
  .dbt-col { color: var(--text-dim); margin-inline-end: var(--sp-2); }
  .dbt-col::after { content: ':'; }
  .dbt-before { color: var(--danger); }
  .dbt-arrow { color: var(--text-dim); padding-inline: var(--sp-2); }
  .dbt-after { color: var(--success); font-weight: 600; }
  .dbt-hidden { color: var(--text-dim); font-style: italic; }
  .dbt-record {
    display: grid; grid-template-columns: minmax(12ch, max-content) minmax(0, 1fr); column-gap: var(--sp-5);
    margin: 0; padding-block: var(--sp-2); padding-inline: calc(var(--sp-4) + 12px + var(--sp-3)) var(--sp-4);
    border-block-end: 1px solid var(--separator); background: var(--surface); font-size: var(--fs-xs);
  }
  .dbt-field { display: contents; }
  .dbt-field dt { color: var(--text-dim); padding-block: 1px; white-space: nowrap; overflow: hidden; text-overflow: ellipsis; max-inline-size: 32ch; }
  .dbt-field dd { margin: 0; padding-block: 1px; color: var(--text); white-space: pre-wrap; word-break: break-word; }
  .dbt-field.is-changed dt, .dbt-field.is-changed dd { background: var(--warning-soft); }
  .dbt-field dd.is-null { color: var(--text-dim); }
  .dbt-show { margin-block: var(--sp-1); margin-inline: var(--sp-4); }
  .dbt-quiet { margin: 0; padding-block: var(--sp-2); padding-inline: var(--sp-4); color: var(--text-dim); font-size: var(--fs-xs); }
</style>
