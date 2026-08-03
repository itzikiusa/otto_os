<script lang="ts">
  // Collapsible, LAZILY-rendered JSON tree.
  //
  // Replaces "stringify the whole value into one <pre>", which is what broke the
  // UI on fat documents: a single `lobby_api.lobby_format_history` doc is ~88KB,
  // so a 100-row result pretty-printed with a <span> per token is millions of DOM
  // nodes — the browser stops responding long before it finishes painting.
  //
  // The invariant that keeps this bounded: a COLLAPSED container renders ONE
  // summary line and recurses into NOTHING. Containers auto-collapse once they're
  // deep or wide, so first paint of an 88KB document is a handful of nodes and the
  // user opens only the branch they care about. Long arrays additionally render in
  // chunks, so expanding a 5,000-element list doesn't undo the win.
  //
  // Text is interpolated (Svelte-escaped), never `{@html}` — unlike the old
  // highlightJsonHtml path this can't emit markup from data.
  import Self from './JsonTree.svelte';
  import { bsonScalar } from './bson';

  interface Props {
    value: unknown;
    /** Object key / array index owning this value; null at the root. */
    label?: string | null;
    depth?: number;
  }
  let { value, label = null, depth = 0 }: Props = $props();

  // Auto-expand only what stays cheap: shallow AND narrow. Everything else opens
  // on click. Tuned so a typical Mongo document shows its top-level shape (and
  // small metadata subdocuments) while blob-ish arrays stay shut.
  const AUTO_DEPTH = 2;
  const AUTO_ITEMS = 20;
  /** Children rendered per "show more" once a container is open. */
  const CHUNK = 50;
  /** Inline string cap — full text stays one click away. */
  const STR_MAX = 200;

  const bson = $derived(bsonScalar(value));
  const isArr = $derived(Array.isArray(value));
  const isObj = $derived(!isArr && value !== null && typeof value === 'object' && bson === null);
  const isContainer = $derived(isArr || isObj);

  const entries = $derived.by<[string, unknown][]>(() => {
    if (isArr) return (value as unknown[]).map((v, i) => [String(i), v]);
    if (isObj) return Object.entries(value as Record<string, unknown>);
    return [];
  });
  const size = $derived(entries.length);

  // Initial disclosure computed WITHOUT reading a $derived during init — the
  // derived graph isn't settled yet at that point and a lazy read here is exactly
  // the shape that trips `state_unsafe_mutation`.
  function initialOpen(): boolean {
    let n = 0;
    if (Array.isArray(value)) n = value.length;
    else if (value !== null && typeof value === 'object' && bsonScalar(value) === null) {
      n = Object.keys(value as object).length;
    } else return false;
    return n > 0 && depth < AUTO_DEPTH && n <= AUTO_ITEMS;
  }
  let open = $state(initialOpen());
  let shown = $state(CHUNK);
  let strOpen = $state(false);

  const visible = $derived(open ? entries.slice(0, shown) : []);
  const hiddenCount = $derived(Math.max(0, size - shown));

  /** One-line summary for a closed container — the whole point of collapsing. */
  const summary = $derived(
    isArr
      ? size === 0
        ? '[]'
        : `[ ${size} ${size === 1 ? 'item' : 'items'} ]`
      : size === 0
        ? '{}'
        : `{ ${size} ${size === 1 ? 'field' : 'fields'} }`,
  );

  const str = $derived(typeof value === 'string' ? value : '');
  const strLong = $derived(str.length > STR_MAX);
</script>

{#if isContainer}
  <div class="node" class:root={depth === 0}>
    {#if size === 0}
      <div class="line">
        {#if label !== null}<span class="k">{label}</span><span class="sep">:</span>{/if}
        <span class="empty">{summary}</span>
      </div>
    {:else}
      <button
        class="line toggle"
        type="button"
        aria-expanded={open}
        onclick={() => (open = !open)}
        title={open ? 'Collapse' : 'Expand'}
      >
        <span class="chev" aria-hidden="true">{open ? '▾' : '▸'}</span>
        {#if label !== null}<span class="k">{label}</span><span class="sep">:</span>{/if}
        <span class="sum" class:dimmed={open}>{summary}</span>
      </button>
      {#if open}
        <div class="kids">
          {#each visible as [k, v] (k)}
            <Self value={v} label={k} depth={depth + 1} />
          {/each}
          {#if hiddenCount > 0}
            <button class="more" type="button" onclick={() => (shown += CHUNK)}>
              show {Math.min(CHUNK, hiddenCount)} more · {hiddenCount} hidden
            </button>
          {/if}
        </div>
      {/if}
    {/if}
  </div>
{:else}
  <div class="line leaf">
    {#if label !== null}<span class="k">{label}</span><span class="sep">:</span>{/if}
    {#if bson !== null}
      <span class="json-bson">{bson}</span>
    {:else if value === null || value === undefined}
      <span class="json-null">null</span>
    {:else if typeof value === 'string'}
      <span class="json-str"
        >"{strLong && !strOpen ? str.slice(0, STR_MAX) : str}{strLong && !strOpen ? '…' : ''}"</span
      >
      {#if strLong}
        <button class="more inline" type="button" onclick={() => (strOpen = !strOpen)}>
          {strOpen ? 'less' : `${str.length - STR_MAX} more chars`}
        </button>
      {/if}
    {:else if typeof value === 'number' || typeof value === 'bigint'}
      <span class="json-num">{value}</span>
    {:else if typeof value === 'boolean'}
      <span class="json-bool">{value}</span>
    {:else}
      <span class="json-str">{String(value)}</span>
    {/if}
  </div>
{/if}

<style>
  .node.root {
    display: block;
  }
  .line {
    display: flex;
    align-items: baseline;
    gap: 4px;
    font-size: 12px;
    line-height: 1.55;
    min-width: 0;
    text-align: left;
  }
  .leaf {
    /* Long scalars wrap rather than force the pane to scroll sideways. */
    flex-wrap: wrap;
    word-break: break-word;
  }
  .toggle {
    width: 100%;
    background: none;
    border: none;
    padding: 0;
    margin: 0;
    color: inherit;
    font: inherit;
    cursor: pointer;
    border-radius: var(--radius-s);
  }
  .toggle:hover {
    background: color-mix(in srgb, var(--text-dim) 10%, transparent);
  }
  .chev {
    color: var(--text-dim);
    width: 10px;
    flex: none;
  }
  .k {
    color: var(--accent);
    font-weight: 600;
  }
  .sep {
    color: var(--text-dim);
    margin-left: -3px;
  }
  .sum {
    color: var(--text-dim);
  }
  .sum.dimmed {
    opacity: 0.55;
  }
  .empty {
    color: var(--text-dim);
  }
  /* Indent guide: children hang off a hairline so deep nesting stays readable. */
  .kids {
    margin-left: 5px;
    padding-left: 9px;
    border-left: 1px solid color-mix(in srgb, var(--text-dim) 22%, transparent);
  }
  .more {
    display: inline-flex;
    align-items: center;
    margin: 2px 0;
    padding: 1px 6px;
    font-size: 11px;
    color: var(--text-dim);
    background: color-mix(in srgb, var(--text-dim) 8%, transparent);
    border: 1px solid var(--border);
    border-radius: var(--radius-s);
    cursor: pointer;
  }
  .more:hover {
    color: var(--text);
  }
  .more.inline {
    margin-left: 4px;
  }
  .json-str {
    color: var(--ok, #3fb950);
  }
  .json-num {
    color: var(--info, #58a6ff);
  }
  .json-bool,
  .json-null {
    color: var(--warn, #d29922);
  }
  .json-bson {
    color: var(--accent);
    font-style: italic;
  }
</style>
