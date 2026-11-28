<script lang="ts">
  // Variable rows for the debugger: the Variables tree and expandable evaluate
  // results share them. Open state is keyed by PATH (owned by the caller, so it
  // survives a step — I2-08), a value that changed since the previous stop is
  // marked, and each row has a menu (I2-13): copy value / name / expression,
  // add to watches, view a long value.
  import Icon from '../../../lib/components/Icon.svelte';
  import Modal from '../../../lib/components/Modal.svelte';
  import { copyWithToast } from '../../../lib/clipboard';
  import { confirmer } from '../../../lib/confirm.svelte';
  import { ctxMenu } from '../../../lib/contextmenu.svelte';
  import { rowMenu } from '../../../lib/rowMenu';
  import { ide, type IdeVariable } from '../ideStore.svelte';
  import { copyExpression, varPath } from './ideVarTree';

  interface Props {
    vars: readonly IdeVariable[];
    /** Path of the parent (a scope name, or an evaluate result's id). */
    root: string;
    /** DAP `variablesReference` of the container holding `vars` (for `setVariable`). */
    parentRef?: number;
    /** Ancestors of `vars` (for "Copy as expression"); empty under a scope. */
    chain?: readonly IdeVariable[];
    openPaths: Readonly<Record<string, boolean>>;
    onsetopen: (path: string, open: boolean) => void;
    changed?: (path: string, value: string) => boolean;
    depth?: number;
  }

  let { vars, root, parentRef = 0, chain = [], openPaths, onsetopen, changed, depth = 0 }: Props = $props();

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

  async function editVariableValue(v: IdeVariable, containerRef: number): Promise<void> {
    if (containerRef <= 0) return;
    const next = await confirmer.promptText(`New value for ${v.name}`, {
      title: 'Set variable value',
      confirmLabel: 'Set value',
      initial: v.value,
      placeholder: v.value,
    });
    if (next == null) return;
    if (await ide.setDebugVariable(containerRef, v.name, next)) void ide.refreshWatches();
  }

  function openMenu(e: MouseEvent, v: IdeVariable, ancestors: readonly IdeVariable[], containerRef: number): void {
    e.preventDefault();
    const expr = copyExpression([...ancestors, v]);
    const canSet = containerRef > 0 && ide.activeDebug?.state === 'paused' && Boolean(ide.activeDebug?.supports_set_variable);
    ctxMenu.show(e, [
      ...(canSet
        ? [
            { label: 'Set value…', icon: 'edit' as const, action: () => void editVariableValue(v, containerRef) },
            { separator: true as const },
          ]
        : []),
      { label: 'Copy value', icon: 'copy', action: () => void copyWithToast(v.value, v.name) },
      { label: 'Copy name', action: () => void copyWithToast(v.name, 'name') },
      { label: 'Copy as expression', action: () => void copyWithToast(expr, 'expression') },
      { separator: true },
      { label: 'Add to watches', icon: 'eye', action: () => ide.addWatch(expr) },
      { label: 'View value…', action: () => (viewing = { name: expr, value: v.value }) },
    ]);
  }
</script>

{#snippet rows(list: readonly IdeVariable[], level: number, parent: string, ancestors: readonly IdeVariable[], containerRef: number)}
  {@const capKey = parent}
  {@const capped = !showAll[capKey] && list.length > VAR_CAP ? list.slice(0, VAR_CAP) : list}
  {#each capped as v (`${v.name}:${v.variables_reference}`)}
    {@const path = varPath(parent, v.name)}
    {@const expandable = v.variables_reference > 0}
    {@const expanded = expandable && Boolean(openPaths[path])}
    {@const children = ide.expandedVariables[v.variables_reference] ?? []}
    {@const isChanged = Boolean(changed?.(path, v.value))}
    <div
      class="side-row dbg-var-row"
      class:is-changed={isChanged}
      style:--side-depth={level}
      role="listitem"
      data-testid="ide-debug-var-row"
      data-var-name={v.name}
    >
      {#if expandable}
        <button
          type="button"
          class="side-twisty"
          tabindex="-1"
          data-side-toggle
          aria-label={expanded ? `Collapse ${v.name}` : `Expand ${v.name}`}
          title={expanded ? `Collapse ${v.name}` : `Expand ${v.name}`}
          onclick={() => void toggle(path, v)}
        ><Icon name={expanded ? 'chevronDown' : 'chevronRight'} size={12} /></button>
      {/if}
      <button
        type="button"
        class="side-row-main"
        data-side-row
        aria-expanded={expandable ? expanded : undefined}
        title={isChanged ? `${v.value} (changed)` : v.value}
        onclick={() => void toggle(path, v)}
        oncontextmenu={(e) => openMenu(e, v, ancestors, containerRef)}
        use:rowMenu
      >
        <span class="dbg-var-name mono" dir="ltr">{v.name}</span>
        <span class="dbg-var-eq dim">=</span>
        <span class="dbg-var-val mono ellipsis" dir="ltr">{v.value}</span>
        {#if v.var_type}<span class="side-row-trailing mono dbg-var-type" dir="ltr" title={v.var_type}>{v.var_type}</span>{/if}
      </button>
      <button type="button" class="icon-btn side-row-more" tabindex="-1" aria-label={`More actions for ${v.name}`} title={`More actions for ${v.name}`} onclick={(e) => openMenu(e, v, ancestors, containerRef)}>
        <Icon name="more" size={14} />
      </button>
    </div>
    {#if expanded}
      {#if loadingRefs[v.variables_reference] && children.length === 0}
        <div class="dbg-var-empty dim" style:padding-inline-start={`${32 + level * 12}px`}>
          <span class="spinner" style="--spinner-size: 12px" aria-hidden="true"></span><span>Inspecting fields…</span>
        </div>
      {:else if children.length === 0}
        <div class="dbg-var-empty dim" style:padding-inline-start={`${32 + level * 12}px`}>Empty</div>
      {:else}
        {@render rows(children, level + 1, path, [...ancestors, v], v.variables_reference)}
      {/if}
    {/if}
  {/each}
  {#if !showAll[capKey] && list.length > VAR_CAP}
    <div class="dbg-var-empty" style:padding-inline-start={`${16 + level * 12}px`}>
      <button type="button" class="btn small ghost" onclick={() => (showAll = { ...showAll, [capKey]: true })}>
        Show all {list.length} items ({list.length - VAR_CAP} more)
      </button>
    </div>
  {/if}
{/snippet}

{@render rows(vars, depth, root, chain, parentRef)}

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
  /* The shared side row (app.css "Side surfaces"): one look with every list and tree. */
  .dbg-var-row .side-row-main { gap: 4px; }
  .dbg-var-name { color: var(--accent-text); font-size: var(--fs-s); flex-shrink: 0; }
  .dbg-var-eq { font-size: var(--fs-s); flex-shrink: 0; }
  .dbg-var-val { flex: 1; min-inline-size: 0; color: var(--text); font-size: var(--fs-s); }
  .dbg-var-row.is-changed .dbg-var-val { color: var(--accent-text); font-weight: 600; }
  .dbg-var-type { max-inline-size: 110px; overflow: hidden; text-overflow: ellipsis; }
  .dbg-var-empty { display: flex; align-items: center; gap: 6px; padding-block: 4px; font-size: var(--fs-xs); }
  .dbg-view-value { margin: 0; max-block-size: 60vh; overflow: auto; white-space: pre-wrap; overflow-wrap: anywhere; font-size: var(--fs-xs); }
</style>
