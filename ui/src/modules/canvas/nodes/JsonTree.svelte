<script lang="ts">
  // Recursive JSON tree row used by JsonNode. Renders a single value; objects and
  // arrays render an expand/collapse caret and recurse into their children. Kept
  // deliberately tiny — this is a node-internal helper, not a public component.
  import { untrack } from 'svelte';
  import Self from './JsonTree.svelte';
  import Icon from '../../../lib/components/Icon.svelte';

  interface Props {
    value: unknown;
    /** Key/index label for this row (null at the root). */
    name: string | number | null;
    depth: number;
  }
  let { value, name, depth }: Props = $props();

  const isArray = $derived(Array.isArray(value));
  const isObject = $derived(value !== null && typeof value === 'object');
  // Auto-expand the first two levels; collapse deeper to keep the node compact.
  // `depth` only seeds the initial open state (then the user toggles it), so read
  // it untracked — this is an intentional one-time capture, not reactive state.
  let open = $state(untrack(() => depth) < 2);

  // Entries for objects/arrays as [key, child] pairs.
  const entries = $derived.by((): [string | number, unknown][] => {
    if (isArray) return (value as unknown[]).map((v, i) => [i, v]);
    if (isObject) return Object.entries(value as Record<string, unknown>);
    return [];
  });
  const count = $derived(entries.length);

  // Short summary shown when collapsed: {…3} or […2].
  const summary = $derived(isArray ? `[ ${count} ]` : `{ ${count} }`);

  function valueClass(v: unknown): string {
    if (v === null) return 'null';
    const t = typeof v;
    if (t === 'string') return 'str';
    if (t === 'number') return 'num';
    if (t === 'boolean') return 'bool';
    return 'other';
  }
  function scalarText(v: unknown): string {
    if (v === null) return 'null';
    if (typeof v === 'string') return `"${v}"`;
    return String(v);
  }
</script>

<div class="row" style:padding-inline-start={`${depth > 0 ? 12 : 0}px`}>
  {#if isObject}
    <button class="caret" onclick={() => (open = !open)} aria-expanded={open}>
      <span class="tw" class:open aria-hidden="true"><Icon name="chevronRight" size={12} /></span>
      {#if name !== null}<span class="key">{name}:</span>{/if}
      {#if !open}<span class="sum">{summary}</span>{/if}
    </button>
    {#if open}
      <div class="children">
        {#each entries as [k, child] (k)}
          <Self value={child} name={k} depth={depth + 1} />
        {/each}
      </div>
    {/if}
  {:else}
    <div class="leaf">
      {#if name !== null}<span class="key">{name}:</span>{/if}
      <span class={valueClass(value)}>{scalarText(value)}</span>
    </div>
  {/if}
</div>

<style>
  .row {
    width: 100%;
  }
  .caret {
    display: flex;
    align-items: center;
    gap: 4px;
    width: 100%;
    background: none;
    border: none;
    padding: 0;
    cursor: pointer;
    color: var(--text);
    font: inherit;
    text-align: start;
  }
  .tw {
    display: inline-flex;
    transition: transform var(--dur-fast) ease;
    color: var(--text-dim);
  }
  .tw.open {
    transform: rotate(90deg);
  }
  /* RTL: the closed chevron points along the reading direction. */
  :global([dir='rtl']) .tw:not(.open) {
    transform: scaleX(-1);
  }
  @media (prefers-reduced-motion: reduce) {
    .tw {
      transition: none;
    }
  }
  .children {
    border-inline-start: 1px solid var(--border);
    margin-inline-start: 4px;
  }
  .leaf {
    display: flex;
    gap: 4px;
    padding-inline-start: 14px;
  }
  .key {
    color: var(--accent-text);
  }
  .sum {
    color: var(--text-dim);
  }
  .str {
    color: var(--success);
  }
  .num {
    color: var(--accent-text);
  }
  .bool {
    color: var(--warning);
  }
  .null,
  .other {
    color: var(--text-dim);
  }
</style>
