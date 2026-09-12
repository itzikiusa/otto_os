<script lang="ts">
  // Quick Mongo filter bar: type a `{ field: value }` object, press Enter, and
  // the active `db.<coll>.find(...)` is rewritten with it (Replace = new
  // filter, AND = merged into the existing one) and run — the one-line answer
  // to "show me the docs where …" without editing the query text by hand.
  // Column chips insert `"col": ` at the caret. Renders nothing unless the
  // statement is a single `find` the splicer can rewrite (`applyMongoFilterObject`).
  import Icon from '../../lib/components/Icon.svelte';
  import { toasts } from '../../lib/toast.svelte';
  import { applyMongoFilterObject, type FilterMode } from './query-filter';

  interface Props {
    /** The statement that PRODUCED the rows on screen (`ran_statement`). */
    statement: string;
    /** Result column names — one chip each. */
    columns: string[];
    onrun: (q: string) => void;
  }
  let { statement, columns, onrun }: Props = $props();

  let text = $state('');
  let mode = $state<FilterMode>('set');
  let inputEl = $state<HTMLInputElement | null>(null);

  // Hidden for aggregates, batches, scripts and SELECT-translated finds — the
  // same gate the rewrite itself applies, probed with an empty filter.
  const visible = $derived(applyMongoFilterObject(statement, '{}', 'set') !== null);

  /** Chips are capped so a 200-field projection doesn't bury the input. */
  const CHIP_MAX = 24;
  const chips = $derived(columns.slice(0, CHIP_MAX));

  function submit(): void {
    const src = text.trim() || '{}';
    const q = applyMongoFilterObject(statement, src, mode);
    if (q === null) {
      toasts.error('Filter must be a { … } object');
      inputEl?.focus();
      return;
    }
    onrun(q);
  }

  /** Insert `"col": ` at the caret (wrapping an empty box in `{ … }` and
   *  separating from a preceding entry with a comma) and refocus. */
  function insertChip(col: string): void {
    const el = inputEl;
    const token = `${JSON.stringify(col)}: `;
    if (!el || text.trim() === '') {
      text = `{ ${token} }`;
      queueMicrotask(() => {
        el?.focus();
        el?.setSelectionRange(text.length - 2, text.length - 2);
      });
      return;
    }
    const start = el.selectionStart ?? text.length;
    const end = el.selectionEnd ?? start;
    const before = text.slice(0, start);
    const sep = /[^\s{,]\s*$/.test(before) ? ', ' : '';
    text = before + sep + token + text.slice(end);
    const caret = before.length + sep.length + token.length;
    queueMicrotask(() => {
      el.focus();
      el.setSelectionRange(caret, caret);
    });
  }

  function onKeydown(e: KeyboardEvent): void {
    if (e.key === 'Enter') {
      e.preventDefault();
      submit();
    } else if (e.key === 'Escape' && text) {
      e.preventDefault();
      text = '';
    }
  }
</script>

{#if visible}
  <div class="mfb" data-testid="mongo-filter-bar">
    <span class="mfb-label" title="Rewrite the active find() filter and run"><Icon name="search" size={11} />Filter</span>
    <input
      bind:this={inputEl}
      class="mfb-input mono"
      type="text"
      placeholder={'{ field: value }'}
      bind:value={text}
      onkeydown={onKeydown}
      spellcheck="false"
      autocomplete="off"
      aria-label="Mongo filter object"
    />
    <div class="mfb-mode" role="radiogroup" aria-label="Filter mode">
      <button class="mfb-seg" class:on={mode === 'set'} role="radio" aria-checked={mode === 'set'} onclick={() => (mode = 'set')} title="Replace the current filter">Replace</button>
      <button class="mfb-seg" class:on={mode === 'and'} role="radio" aria-checked={mode === 'and'} onclick={() => (mode = 'and')} title="Merge into the current filter">AND</button>
    </div>
    <button class="mfb-run" onclick={submit} title="Rewrite the find() filter and run (Enter)"><Icon name="play" size={10} />Run</button>
    {#if chips.length > 0}
      <div class="mfb-chips" aria-label="Insert a field">
        {#each chips as col (col)}
          <button class="mfb-chip mono" onclick={() => insertChip(col)} title={`Insert "${col}": at the caret`}>{col}</button>
        {/each}
        {#if columns.length > chips.length}<span class="mfb-more">+{columns.length - chips.length}</span>{/if}
      </div>
    {/if}
  </div>
{/if}

<style>
  .mfb {
    display: flex;
    align-items: center;
    flex-wrap: wrap;
    gap: 6px;
    padding: 4px 8px;
    border-bottom: 1px solid var(--border);
    background: color-mix(in srgb, var(--surface-2) 60%, transparent);
    font-size: 11.5px;
  }
  .mfb-label {
    display: inline-flex;
    align-items: center;
    gap: 4px;
    color: var(--text-dim);
    font-weight: 600;
  }
  .mfb-input {
    flex: 1 1 220px;
    min-width: 0;
    height: 22px;
    padding: 0 8px;
    font-size: 11.5px;
    border: 1px solid var(--border);
    border-radius: var(--radius-s);
    background: var(--surface);
    color: var(--text);
  }
  .mfb-input:focus {
    outline: none;
    border-color: var(--accent);
  }
  /* Two-way segmented toggle (mirrors the results view switcher). */
  .mfb-mode {
    display: inline-flex;
    border: 1px solid var(--border);
    border-radius: var(--radius-s);
    overflow: hidden;
  }
  .mfb-seg {
    height: 20px;
    padding: 0 8px;
    border: none;
    background: transparent;
    color: var(--text-dim);
    font-size: 11px;
    cursor: pointer;
  }
  .mfb-seg + .mfb-seg {
    border-left: 1px solid var(--border);
  }
  .mfb-seg.on {
    background: var(--surface);
    color: var(--accent);
    font-weight: 600;
  }
  .mfb-run {
    display: inline-flex;
    align-items: center;
    gap: 4px;
    height: 22px;
    padding: 0 9px;
    border-radius: var(--radius-s);
    border: 1px solid var(--border);
    background: var(--surface-2);
    color: var(--text);
    font-size: 11.5px;
    cursor: pointer;
  }
  .mfb-run:hover {
    border-color: color-mix(in srgb, var(--accent) 45%, transparent);
    color: var(--accent);
  }
  .mfb-chips {
    flex: 1 0 100%;
    display: flex;
    flex-wrap: wrap;
    gap: 4px;
    align-items: center;
  }
  .mfb-chip {
    height: 18px;
    padding: 0 7px;
    border-radius: 999px;
    border: 1px solid var(--border);
    background: transparent;
    color: var(--text-dim);
    font-size: 10.5px;
    cursor: pointer;
  }
  .mfb-chip:hover {
    color: var(--accent);
    border-color: color-mix(in srgb, var(--accent) 45%, transparent);
  }
  .mfb-more {
    color: var(--text-dim);
    font-size: 10.5px;
  }
</style>
