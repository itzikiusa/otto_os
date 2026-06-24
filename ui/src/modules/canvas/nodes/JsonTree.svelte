<script lang="ts">
  // Recursive JSON tree row used by JsonNode. Renders a single value; objects and
  // arrays render an expand/collapse caret and recurse into their children. Kept
  // deliberately tiny — this is a node-internal helper, not a public component.
  import { untrack } from 'svelte';
  import Self from './JsonTree.svelte';

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

<div class="row" style:padding-left={`${depth > 0 ? 12 : 0}px`}>
  {#if isObject}
    <button class="caret" onclick={() => (open = !open)} aria-expanded={open}>
      <span class="tw" class:open>▸</span>
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
    text-align: left;
  }
  .tw {
    display: inline-block;
    transition: transform 120ms ease;
    color: var(--text-dim);
    font-size: 10px;
  }
  .tw.open {
    transform: rotate(90deg);
  }
  .children {
    border-left: 1px solid var(--border);
    margin-left: 4px;
  }
  .leaf {
    display: flex;
    gap: 4px;
    padding-left: 14px;
  }
  .key {
    color: var(--accent);
  }
  .sum {
    color: var(--text-dim);
  }
  .str {
    color: var(--status-working);
  }
  .num {
    color: var(--accent);
  }
  .bool {
    color: var(--status-warn);
  }
  .null,
  .other {
    color: var(--text-dim);
  }
</style>
