<script lang="ts">
  // One value of a Vertical-view record as nested `field: value` rows — the
  // sub-document counterpart of JsonTree, keyed to the same expansion plan.
  //
  // The invariant that keeps a fat document bounded: a CLOSED container is ONE
  // summary row (`.vsum`) and recurses into nothing; open containers render
  // their children under a `.vnest` in CHUNK slices with a "show more". What
  // is open comes from the owner's `plan` (a node budget with sticky per-path
  // overrides — see expansion-plan.ts); a toggle records an override and the
  // owner re-plans every drawn record.
  //
  // Editing (Vertical view only): double-click a scalar leaf → inline typed
  // editor → `flow.parkPath` (a `$set` on the dotted path for Mongo; folded
  // into the column's cell draft for SQL). Rows with a pending op get the
  // grid's amber `dirty` look; a container with a change somewhere inside it
  // gets a thinner `dirty-in` marker; a pending `$set` on a path that doesn't
  // exist yet renders as a phantom row under its parent.
  import Self from './VerticalTree.svelte';
  import ValueEditor from './ValueEditor.svelte';
  import { bsonScalar } from './bson';
  import type { DbEngine } from '../../lib/api/types';
  import { typedRaw, type EditFlow } from './EditFlow.svelte';
  import type { FieldCtx, TypedValue } from './edit-types';
  import { CHUNK, entriesOf, setOverride, valueKind, type ExpansionState } from './expansion-plan';
  import { cellStr } from './results-format';

  interface Props {
    value: unknown;
    /** Dotted path from the record root; the column name for a top-level row. */
    path: string;
    depth: number;
    /** Key / index shown as the row label (the column name at the top). */
    label: string;
    expansion: ExpansionState;
    plan: Set<string>;
    editable: boolean;
    rowIdx: number;
    /** Column the path lives in; -1 for a phantom top-level field (Mongo). */
    colIdx: number;
    flow: EditFlow;
    engine: DbEngine | null;
    /** The top-level value IS a parked cell draft (rendered as its tree). */
    cellDraft?: boolean;
    onfieldmenu: (e: MouseEvent, ctx: FieldCtx) => void;
  }
  let {
    value,
    path,
    depth,
    label,
    expansion,
    plan,
    editable,
    rowIdx,
    colIdx,
    flow,
    engine,
    cellDraft = false,
    onfieldmenu,
  }: Props = $props();

  /** Inline string cap — full text stays one click away. */
  const STR_MAX = 200;

  const bson = $derived(bsonScalar(value));
  const entries = $derived(entriesOf(value));
  const container = $derived(bson === null && value !== null && typeof value === 'object');
  const size = $derived(entries.length);

  // What is parked at exactly this path (a `$set` typed value, an `$unset`, a
  // rename, or — at the top — the column's cell draft).
  const pend = $derived(flow.pendingAt(rowIdx, path, colIdx));
  const pendTyped = $derived<TypedValue | null>(
    pend !== undefined && pend !== 'unset' && 'kind' in pend ? pend : null,
  );
  const renamedTo = $derived(pend !== undefined && pend !== 'unset' && 'renamedTo' in pend ? pend.renamedTo : null);
  const dirty = $derived(pend !== undefined);
  // A container whose whole value is being replaced / removed shows what will
  // be written (one dirty leaf), not the stale subtree. A cell draft is the
  // opposite case: the value IS the draft, so it renders as a tree.
  const asLeaf = $derived(
    !container || size === 0 || pend === 'unset' || (pendTyped !== null && !cellDraft),
  );
  const dirtyInside = $derived(!asLeaf && !dirty && flow.hasPendingUnder(rowIdx, path));

  const open = $derived(plan.has(path));
  let shown = $state(CHUNK);
  const visible = $derived(open ? entries.slice(0, shown) : []);
  const hiddenCount = $derived(Math.max(0, size - shown));
  /** Pending `$set`s for keys this container doesn't have yet (phantom rows). */
  const phantoms = $derived(
    open ? flow.pendingSetUnder(rowIdx, path).filter(([k]) => !entries.some(([ek]) => ek === k)) : [],
  );
  const isArr = $derived(Array.isArray(value));
  const summary = $derived(
    isArr
      ? size === 0
        ? '[]'
        : `[ ${size} ${size === 1 ? 'item' : 'items'} ]`
      : size === 0
        ? '{}'
        : `{ ${size} ${size === 1 ? 'field' : 'fields'} }`,
  );

  // Leaf display: typed BSON label, ∅ for null/absent, clipped long strings.
  const text = $derived(value === null || value === undefined ? '' : cellStr(value));
  const strLong = $derived(typeof value === 'string' && value.length > STR_MAX);
  let strOpen = $state(false);
  const inArray = $derived(/(^|\.)\d+(\.|$)/.test(path));
  const topLevel = $derived(!path.includes('.'));

  /** How a parked typed value reads in the row (what the statement will write). */
  function pendingText(tv: TypedValue): string {
    switch (tv.kind) {
      case 'null':
        return '∅';
      case 'objectId':
        return `ObjectId("${tv.raw}")`;
      case 'date':
        return `ISODate("${tv.raw}")`;
      case 'long':
        return `NumberLong("${tv.raw}")`;
      default:
        return tv.raw === '' ? "''" : tv.raw;
    }
  }

  const canEdit = $derived(editable && !flow.reviewSql && (colIdx === -1 || flow.isEditableCell(colIdx)));
  const editing = $derived(
    flow.pathEditing !== null && flow.pathEditing.rowIdx === rowIdx && flow.pathEditing.path === path,
  );
  // The editor pre-fills from the parked value when there is one, else the
  // stored value — so re-editing a dirty row resumes the draft.
  const editKind = $derived(pendTyped ? pendTyped.kind : valueKind(value));
  const editRaw = $derived(pendTyped ? pendTyped.raw : typedRaw(value));

  function beginEdit(): void {
    if (!canEdit || !asLeaf) return;
    flow.beginPathEdit(rowIdx, colIdx, path);
  }
  function ctx(): FieldCtx {
    return { rowIdx, colIdx, path, label, value, container: container && size > 0, topLevel, inArray, edit: beginEdit };
  }
  function toggle(): void {
    setOverride(expansion, path, !open);
  }
</script>

{#if asLeaf}
  <!-- svelte-ignore a11y_no_static_element_interactions -->
  <div
    class="vrow"
    class:dirty
    class:editable={canEdit}
    style="--depth:{depth}"
    title={dirty ? 'Pending change — Review & apply (bar below) writes it' : canEdit ? 'Double-click to edit' : undefined}
    ondblclick={beginEdit}
    oncontextmenu={(e) => onfieldmenu(e, ctx())}
  >
    <span class="vk mono" title={path}>{label}{#if renamedTo !== null}<span class="vk-ren"> → {renamedTo}</span>{/if}</span>
    {#if editing}
      <span class="vv mono edit"><ValueEditor kind={editKind} raw={editRaw} {engine} onsave={(tv) => flow.parkPath(rowIdx, colIdx, path, tv)} oncancel={() => flow.cancelPathEdit()} /></span>
    {:else if pend === 'unset'}
      <span class="vv mono pend"><s>{text}</s> <em>unset</em></span>
    {:else if pendTyped !== null}
      <span class="vv mono pend" class:null-glyph={pendTyped.kind === 'null'}>{pendingText(pendTyped)}</span>
    {:else if value === null || value === undefined}
      <span class="vv mono null-glyph">∅</span>
    {:else if bson !== null}
      <span class="vv mono bson">{bson}</span>
    {:else if strLong}
      <span class="vv mono tree">{strOpen ? text : text.slice(0, STR_MAX) + '…'}<button class="vmore inline" type="button" onclick={() => (strOpen = !strOpen)}>{strOpen ? 'less' : `${text.length - STR_MAX} more chars`}</button></span>
    {:else if container}
      <span class="vv mono dim">{summary}</span>
    {:else}
      <span class="vv mono">{text}</span>
    {/if}
  </div>
{:else}
  <!-- svelte-ignore a11y_no_static_element_interactions -->
  <div
    class="vrow ctr"
    class:dirty-in={dirtyInside}
    class:dirty
    style="--depth:{depth}"
    oncontextmenu={(e) => onfieldmenu(e, ctx())}
  >
    <span class="vk mono" title={path}>{label}{#if renamedTo !== null}<span class="vk-ren"> → {renamedTo}</span>{/if}</span>
    <span class="vv mono tree">
      <button class="vsum" type="button" aria-expanded={open} onclick={toggle} title={open ? 'Collapse' : 'Expand'}>
        <span class="chev" aria-hidden="true">{open ? '▾' : '▸'}</span><span class:dimmed={open}>{summary}</span>
      </button>
    </span>
  </div>
  {#if open}
    <div class="vnest" style="--depth:{depth + 1}">
      {#each visible as [k, v] (k)}
        <Self value={v} path={`${path}.${k}`} depth={depth + 1} label={k} {expansion} {plan} {editable} {rowIdx} {colIdx} {flow} {engine} {onfieldmenu} />
      {/each}
      {#each phantoms as [k] (k)}
        <Self value={undefined} path={`${path}.${k}`} depth={depth + 1} label={k} {expansion} {plan} {editable} {rowIdx} {colIdx} {flow} {engine} {onfieldmenu} />
      {/each}
      {#if hiddenCount > 0}
        <button class="vmore" type="button" onclick={() => (shown += CHUNK)}>
          show {Math.min(CHUNK, hiddenCount)} more · {hiddenCount} hidden
        </button>
      {/if}
    </div>
  {/if}
{/if}

<style>
  .vrow {
    display: grid;
    grid-template-columns: minmax(120px, 0.3fr) 1fr;
    gap: 10px;
    padding: 3px 8px;
    font-size: 12px;
    min-width: 0;
  }
  .vrow:hover {
    background: color-mix(in srgb, var(--text-dim) 6%, transparent);
  }
  .vrow.editable {
    cursor: text;
  }
  /* A parked (pending) change: the grid's amber `.cell.dirty` look. */
  .vrow.dirty {
    background: color-mix(in srgb, var(--status-warn) 14%, transparent);
    box-shadow: inset 0 0 0 1px color-mix(in srgb, var(--status-warn) 55%, transparent);
    font-style: italic;
  }
  /* Something inside this container is pending. */
  .vrow.dirty-in {
    box-shadow: inset 2px 0 0 color-mix(in srgb, var(--status-warn) 70%, transparent);
  }
  .vk {
    color: var(--text-dim);
    font-weight: 600;
    overflow-wrap: anywhere;
  }
  .vk-ren {
    color: var(--status-warn);
    font-weight: 500;
  }
  .vv {
    color: var(--text);
    word-break: break-word;
    white-space: pre-wrap;
    min-width: 0;
  }
  /* A tree child manages its own layout — pre-wrap here would turn the markup's
     indentation into stray blank lines. */
  .vv.tree,
  .vv.edit {
    white-space: normal;
  }
  .vv.null-glyph,
  .vv.dim {
    color: var(--text-dim);
  }
  .vv.bson {
    color: var(--accent);
    font-style: italic;
  }
  .vv.pend em {
    color: var(--status-warn);
    font-style: normal;
    font-size: 10.5px;
    margin-left: 6px;
  }
  .vsum {
    display: inline-flex;
    align-items: baseline;
    gap: 4px;
    padding: 0 4px 0 0;
    margin: 0;
    background: none;
    border: none;
    color: var(--text-dim);
    font: inherit;
    cursor: pointer;
    border-radius: var(--radius-s);
  }
  .vsum:hover {
    color: var(--text);
    background: color-mix(in srgb, var(--text-dim) 10%, transparent);
  }
  .vsum .dimmed {
    opacity: 0.55;
  }
  .chev {
    width: 10px;
    flex: none;
  }
  /* Indent guide: nested rows hang off a hairline so depth stays readable. */
  .vnest {
    margin-left: 14px;
    border-left: 1px solid color-mix(in srgb, var(--text-dim) 22%, transparent);
  }
  .vmore {
    display: inline-flex;
    align-items: center;
    margin: 2px 8px;
    padding: 1px 6px;
    font-size: 11px;
    color: var(--text-dim);
    background: color-mix(in srgb, var(--text-dim) 8%, transparent);
    border: 1px solid var(--border);
    border-radius: var(--radius-s);
    cursor: pointer;
  }
  .vmore:hover {
    color: var(--text);
  }
  .vmore.inline {
    margin: 0 0 0 4px;
  }

  /* Vertical / JSON views are the comfiest on a narrow phone — bump them too. */
  @media (max-width: 640px) {
    .vk,
    .vv {
      font-size: 13px;
    }
  }
</style>
