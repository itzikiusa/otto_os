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
  //
  // Two disclosure modes. Standalone (no `plan`): the depth/width heuristic
  // below, local open state — the cell viewer, AWS views, dashboards.
  // Controlled (`plan` + `expansion` given, the JSON result view): a node is
  // open iff its dotted `path` is in the owner's plan (a node budget with
  // sticky per-path overrides, see expansion-plan.ts); toggling records an
  // override on the shared state and the owner re-plans every record.
  //
  // ⌘F (json-find.ts): every line carries `data-jpath`; text the find model
  // leaves out (chevrons, separators, summaries, array-index labels, "more"
  // buttons) is `data-find-skip`. A find reveal (`reveal`, per record) forces
  // paths open, grows "show more" slices and unclips strings on top of the plan.
  import Self from './JsonTree.svelte';
  import { bsonScalar } from './bson';
  import { setOverride, stickyKey, type ExpansionState } from './expansion-plan';
  import { searchableLabel, STR_MAX, type RevealState } from './json-find';

  interface Props {
    value: unknown;
    /** Object key / array index owning this value; null at the root. */
    label?: string | null;
    depth?: number;
    /** Dotted path from the record root ('' at the root); controlled mode only. */
    path?: string;
    /** Open container paths of this record (controlled mode). */
    plan?: Set<string>;
    /** Shared expansion state the toggles write to (controlled mode). */
    expansion?: ExpansionState;
    /** This record's find-reveal overlay (JsonView only). */
    reveal?: RevealState;
  }
  let { value, label = null, depth = 0, path = '', plan, expansion, reveal }: Props = $props();

  // Auto-expand only what stays cheap: shallow AND narrow. Everything else opens
  // on click. Tuned so a typical Mongo document shows its top-level shape (and
  // small metadata subdocuments) while blob-ish arrays stay shut.
  const AUTO_DEPTH = 2;
  const AUTO_ITEMS = 20;
  /** Children rendered per "show more" once a container is open. */
  const CHUNK = 50;

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
    // Controlled mode: the record root is not a planned node — it is always
    // open (its toggle stays local); every other node follows the plan.
    if (plan) return path === '';
    let n = 0;
    if (Array.isArray(value)) n = value.length;
    else if (value !== null && typeof value === 'object' && bsonScalar(value) === null) {
      n = Object.keys(value as object).length;
    } else return false;
    return n > 0 && depth < AUTO_DEPTH && n <= AUTO_ITEMS;
  }
  let localOpen = $state(initialOpen());
  const controlled = $derived(!!plan && path !== '');
  // An override-opened path the planner never reached (a child past its
  // parent's first CHUNK, drawn by "show more") is open too — the plan only
  // queues the first slice.
  const open = $derived(
    (controlled ? plan!.has(path) || expansion?.overrides.get(stickyKey(path)) === true : localOpen) ||
      !!reveal?.opens.has(path),
  );
  function toggle(): void {
    const next = !open;
    reveal?.opens.delete(path);
    if (controlled && expansion) setOverride(expansion, path, next);
    else localOpen = next;
  }
  let shownLocal = $state(CHUNK);
  const shown = $derived(Math.max(shownLocal, reveal?.shown.get(path) ?? 0));
  let strOpenLocal = $state(false);
  const strOpen = $derived(strOpenLocal || !!reveal?.strs.has(path));
  function toggleStr(): void {
    strOpenLocal = !strOpen;
    reveal?.strs.delete(path);
  }
  const keySkip = $derived(label !== null && !searchableLabel(label));

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
      <div class="line" data-jpath={path}>
        {#if label !== null}<span class="k" data-find-skip={keySkip || undefined}>{label}</span><span class="sep" data-find-skip>:</span>{/if}
        <span class="empty">{summary}</span>
      </div>
    {:else}
      <button
        class="line toggle"
        type="button"
        aria-expanded={open}
        onclick={toggle}
        title={open ? 'Collapse' : 'Expand'}
        data-jpath={path}
      >
        <span class="chev" aria-hidden="true" data-find-skip>{open ? '▾' : '▸'}</span>
        {#if label !== null}<span class="k" data-find-skip={keySkip || undefined}>{label}</span><span class="sep" data-find-skip>:</span>{/if}
        <span class="sum" class:dimmed={open} data-find-skip>{summary}</span>
      </button>
      {#if open}
        <div class="kids">
          {#each visible as [k, v] (k)}
            <Self value={v} label={k} depth={depth + 1} path={path ? `${path}.${k}` : k} {plan} {expansion} {reveal} />
          {/each}
          {#if hiddenCount > 0}
            <button class="more" type="button" onclick={() => (shownLocal = shown + CHUNK)} data-find-skip>
              show {Math.min(CHUNK, hiddenCount)} more · {hiddenCount} hidden
            </button>
          {/if}
        </div>
      {/if}
    {/if}
  </div>
{:else}
  <div class="line leaf" data-jpath={path}>
    {#if label !== null}<span class="k" data-find-skip={keySkip || undefined}>{label}</span><span class="sep" data-find-skip>:</span>{/if}
    {#if bson !== null}
      <span class="json-bson">{bson}</span>
    {:else if value === null || value === undefined}
      <span class="json-null">null</span>
    {:else if typeof value === 'string'}
      <span class="json-str"
        >"{strLong && !strOpen ? str.slice(0, STR_MAX) : str}{strLong && !strOpen ? '…' : ''}"</span
      >
      {#if strLong}
        <button class="more inline" type="button" onclick={toggleStr} data-find-skip>
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
    font-size: var(--fs-s);
    line-height: 1.55;
    min-width: 0;
    text-align: start;
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
    color: var(--accent-text);
    font-weight: 600;
  }
  .sep {
    color: var(--text-dim);
    margin-inline-start: -3px;
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
    margin-inline-start: 5px;
    padding-inline-start: 9px;
    border-inline-start: 1px solid color-mix(in srgb, var(--text-dim) 22%, transparent);
  }
  .more {
    display: inline-flex;
    align-items: center;
    margin: 2px 0;
    padding: 1px 6px;
    font-size: var(--fs-xs);
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
    margin-inline-start: 4px;
  }
  .json-str {
    color: var(--success);
  }
  .json-num {
    color: var(--info);
  }
  .json-bool,
  .json-null {
    color: var(--warning);
  }
  .json-bson {
    color: var(--accent-text);
    font-style: italic;
  }
</style>
