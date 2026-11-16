<script lang="ts">
  // Variable rows for the debugger: the Variables tree and expandable evaluate
  // results share them. Open state is keyed by PATH (owned by the caller, so it
  // survives a step — I2-08), a value that changed since the previous stop is
  // marked, and each row has a menu (I2-13): copy value / name / expression,
  // add to watches, view a long value.
  import Icon from '../../../lib/components/Icon.svelte';
  import Modal from '../../../lib/components/Modal.svelte';
  import { copyWithToast } from '../../../lib/clipboard';
  import { ctxMenu } from '../../../lib/contextmenu.svelte';
  import { rowMenu } from '../../../lib/rowMenu';
  import { ide, type IdeVariable } from '../ideStore.svelte';
  import { copyExpression, varPath } from './ideVarTree';

  interface Props {
    vars: readonly IdeVariable[];
    /** Path of the parent (a scope name, or an evaluate result's id). */
    root: string;
    /** Ancestors of `vars` (for "Copy as expression"); empty under a scope. */
    chain?: readonly IdeVariable[];
    openPaths: Readonly<Record<string, boolean>>;
    onsetopen: (path: string, open: boolean) => void;
    changed?: (path: string, value: string) => boolean;
    depth?: number;
  }

  let { vars, root, chain = [], openPaths, onsetopen, changed, depth = 0 }: Props = $props();

  const VAR_CAP = 100;
  let loadingRefs = $state<Record<number, boolean>>({});
  let showAll = $state<Record<string, boolean>>({});
  let viewing = $state<{ name: string; value: string } | null>(null);

  async function toggle(path: string, v: IdeVariable): Promise<void> {
    const ref = v.variables_reference;
    if (ref <= 0) return;
    const next = !openPaths[path];
    onsetopen(path, next);
    if (!next || ide.expandedVariables[ref] || loadingRefs[ref]) return;
    loadingRefs = { ...loadingRefs, [ref]: true };
    try { await ide.expandVariables(ref); }
    finally { loadingRefs = { ...loadingRefs, [ref]: false }; }
  }

  function openMenu(e: MouseEvent, v: IdeVariable, ancestors: readonly IdeVariable[]): void {
    e.preventDefault();
    const expr = copyExpression([...ancestors, v]);
    ctxMenu.show(e, [
      { label: 'Copy value', icon: 'copy', action: () => void copyWithToast(v.value, v.name) },
      { label: 'Copy name', action: () => void copyWithToast(v.name, 'name') },
      { label: 'Copy as expression', action: () => void copyWithToast(expr, 'expression') },
      { separator: true },
      { label: 'Add to watches', icon: 'eye', action: () => ide.addWatch(expr) },
      { label: 'View value…', action: () => (viewing = { name: expr, value: v.value }) },
    ]);
  }
</script>

{#snippet rows(list: readonly IdeVariable[], level: number, parent: string, ancestors: readonly IdeVariable[])}
  {@const capKey = parent}
  {@const capped = !showAll[capKey] && list.length > VAR_CAP ? list.slice(0, VAR_CAP) : list}
  {#each capped as v (`${v.name}:${v.variables_reference}`)}
    {@const path = varPath(parent, v.name)}
    {@const expandable = v.variables_reference > 0}
    {@const expanded = expandable && Boolean(openPaths[path])}
    {@const children = ide.expandedVariables[v.variables_reference] ?? []}
    {@const isChanged = Boolean(changed?.(path, v.value))}
    <div
      class="dbg-var-row reveal-host"
      class:is-changed={isChanged}
      style:padding-inline-start={`${12 + level * 14}px`}
      role="listitem"
      oncontextmenu={(e) => openMenu(e, v, ancestors)}
      use:rowMenu
      data-testid="ide-debug-var-row"
      data-var-name={v.name}
    >
      {#if expandable}
        <button
          type="button"
          class="icon-btn dbg-twisty"
          aria-expanded={expanded}
          aria-label={expanded ? `Collapse ${v.name}` : `Expand ${v.name}`}
          title={expanded ? `Collapse ${v.name}` : `Expand ${v.name}`}
          onclick={() => void toggle(path, v)}
        ><Icon name={expanded ? 'chevronDown' : 'chevronRight'} size={12} /></button>
      {:else}
        <span class="dbg-twisty-spacer" aria-hidden="true"></span>
      {/if}
      <span class="dbg-var-name mono" dir="ltr">{v.name}</span>
      <span class="dbg-var-eq dim">=</span>
      <span class="dbg-var-val mono ellipsis" dir="ltr" title={isChanged ? `${v.value} (changed)` : v.value}>{v.value}</span>
      {#if v.var_type}<span class="dbg-var-type mono ellipsis" dir="ltr" title={v.var_type}>{v.var_type}</span>{/if}
      <button type="button" class="icon-btn reveal-on-hover" aria-label={`More actions for ${v.name}`} title={`More actions for ${v.name}`} onclick={(e) => openMenu(e, v, ancestors)}>
        <Icon name="more" size={12} />
      </button>
    </div>
    {#if expanded}
      {#if loadingRefs[v.variables_reference] && children.length === 0}
        <div class="dbg-var-empty dim" style:padding-inline-start={`${26 + level * 14}px`}>
          <span class="spinner" style="--spinner-size: 12px" aria-hidden="true"></span><span>Inspecting fields…</span>
        </div>
      {:else if children.length === 0}
        <div class="dbg-var-empty dim" style:padding-inline-start={`${26 + level * 14}px`}>Empty</div>
      {:else}
        {@render rows(children, level + 1, path, [...ancestors, v])}
      {/if}
    {/if}
  {/each}
  {#if !showAll[capKey] && list.length > VAR_CAP}
    <div class="dbg-var-empty" style:padding-inline-start={`${12 + level * 14}px`}>
      <button type="button" class="btn small ghost" onclick={() => (showAll = { ...showAll, [capKey]: true })}>
        Show all {list.length} items ({list.length - VAR_CAP} more)
      </button>
    </div>
  {/if}
{/snippet}

{@render rows(vars, depth, root, chain)}

{#if viewing}
  {@const v = viewing}
  <Modal title={`Value of ${v.name}`} width={640} onclose={() => (viewing = null)}>
    <pre class="dbg-view-value mono" dir="ltr" data-testid="ide-debug-view-value">{v.value}</pre>
    {#snippet footer()}
      <button type="button" class="btn" onclick={() => void copyWithToast(v.value, v.name)}><Icon name="copy" size={12} /><span>Copy</span></button>
      <button type="button" class="btn primary" onclick={() => (viewing = null)}>Close</button>
    {/snippet}
  </Modal>
{/if}

<style>
  .dbg-var-row { display: flex; align-items: center; gap: 4px; min-block-size: 24px; padding-block: 2px; padding-inline: 12px 6px; font-size: var(--fs-xs); }
  .dbg-var-row:hover { background: var(--hover); }
  .dbg-twisty { inline-size: 18px; block-size: 18px; }
  .dbg-twisty-spacer { inline-size: 18px; flex-shrink: 0; }
  .dbg-var-name { color: var(--accent-text); font-size: var(--fs-xs); flex-shrink: 0; }
  .dbg-var-eq { font-size: var(--fs-xs); flex-shrink: 0; }
  .dbg-var-val { flex: 1; min-inline-size: 0; color: var(--text); font-size: var(--fs-xs); }
  .dbg-var-row.is-changed .dbg-var-val { color: var(--accent-text); font-weight: 600; }
  .dbg-var-type { color: var(--text-dim); font-size: var(--fs-xs); max-inline-size: 110px; }
  .dbg-var-empty { display: flex; align-items: center; gap: 6px; padding: 4px 24px; font-size: var(--fs-xs); }
  .dbg-view-value { margin: 0; max-block-size: 60vh; overflow: auto; white-space: pre-wrap; overflow-wrap: anywhere; font-size: var(--fs-xs); }
  @media (pointer: coarse) { .dbg-var-row { min-block-size: var(--hit-min); } }
</style>
