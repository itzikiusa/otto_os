<script lang="ts">
  // Site Studio inspector — a "links" field (nav, footer, button rows): a
  // label and a URL per row. Rows are keyed by a client id, not their index,
  // so a removed row takes its own inputs with it (focus and caret stay on the
  // row they were in); each row's controls carry that row's name; and Remove
  // moves focus to the Remove now in its place (else the previous row's, else
  // "Add link") instead of dropping it to the page.
  import { tick } from 'svelte';
  import Icon from '../../../lib/components/Icon.svelte';
  import type { SiteLink } from './engine/types';

  interface Props {
    label: string;
    rows: SiteLink[];
    readonly: boolean;
    onchange: (rows: SiteLink[]) => void;
  }
  let { label, rows, readonly, onchange }: Props = $props();

  let seq = 0;
  let keys: number[] = [];
  /** One stable key per row; an outside change of length (undo, an agent
   *  edit) re-keys, an edit made here keeps the keys in step. */
  const keyed = $derived.by(() => {
    if (keys.length !== rows.length) keys = rows.map(() => ++seq);
    return rows.map((l, i) => ({ l, key: keys[i] }));
  });
  let listEl = $state<HTMLElement | null>(null);

  const rowName = (l: SiteLink, i: number): string => l.label.trim() || `link ${i + 1}`;
  function set(i: number, part: 'label' | 'href', value: string): void {
    onchange(rows.map((l, j) => (j === i ? { ...l, [part]: value } : l)));
  }
  function add(): void {
    keys = [...keys, ++seq];
    onchange([...rows, { label: 'New link', href: '#' }]);
  }
  async function remove(i: number): Promise<void> {
    keys = keys.filter((_, j) => j !== i);
    onchange(rows.filter((_, j) => j !== i));
    await tick();
    const removes = listEl?.querySelectorAll<HTMLElement>('[data-remove-row]') ?? [];
    (removes[Math.min(i, removes.length - 1)] ?? listEl?.querySelector<HTMLElement>('[data-add-row]'))?.focus();
  }
</script>

<div class="field wide" role="group" aria-label={label} bind:this={listEl}>
  <span class="lbl">{label}</span>
  {#each keyed as { l, key }, i (key)}
    <div class="linkrow">
      <input dir="auto" class="input lf-input" value={l.label} aria-label={`Label of ${rowName(l, i)}`} disabled={readonly} oninput={(e) => set(i, 'label', e.currentTarget.value)} />
      <input dir="ltr" class="input lf-input" value={l.href} aria-label={`URL of ${rowName(l, i)}`} disabled={readonly} oninput={(e) => set(i, 'href', e.currentTarget.value)} />
      <button class="icon-btn" data-remove-row onclick={() => void remove(i)} disabled={readonly} aria-label={`Remove link ${rowName(l, i)}`} title={`Remove link ${rowName(l, i)}`}><Icon name="x" size={12} /></button>
    </div>
  {/each}
  {#if !readonly}<button class="btn small ghost" data-add-row onclick={add}><Icon name="plus" size={12} /> Add link</button>{/if}
</div>

<style>
  /* Mirrors the inspector's field rhythm (Inspector.svelte). */
  .field {
    margin-block-end: 10px;
  }
  .lbl {
    font-size: var(--fs-s);
    font-weight: 500;
    color: var(--text-dim);
  }
  .field .lf-input {
    width: 100%;
    min-width: 0;
  }
  .linkrow {
    display: grid;
    grid-template-columns: 1fr 1.2fr auto;
    gap: 4px;
    margin-block-end: 4px;
  }
</style>
