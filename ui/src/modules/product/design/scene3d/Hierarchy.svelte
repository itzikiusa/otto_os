<script lang="ts">
  // Scene hierarchy — the game-studio left panel while a scene3d artifact is open
  // (the arena toggles it in place of the asset list). Groups collapse, rows
  // select, double-click / F2 renames inline, the eye hides, the ⋯ / right-click
  // menu duplicates, deletes, moves to a group or ungroups. The Add ▾ menu offers
  // primitives, lights, a group and "Import GLB…" (the upload itself is the
  // arena's — `onimportGlb`). Every edit is an `ops.ts` call → `onchange(newDoc)`.
  import { ctxMenu, type MenuItem } from '../../../../lib/contextmenu.svelte';
  import Icon from '../../../../lib/components/Icon.svelte';
  import { LIGHT_TYPES, PRIMITIVE_TYPES, type LightType, type PrimitiveType, type Scene3dDoc } from './types';
  import {
    addGroup,
    addLight,
    addPrimitive,
    duplicate,
    findNode,
    isEffectivelyVisible,
    moveToGroup,
    parentGroup,
    remove,
    rename,
    reorder,
    setVisible,
    type Scene3dNode,
  } from './ops';

  interface Props {
    doc: Scene3dDoc;
    selectedId?: string | null;
    onchange: (doc: Scene3dDoc) => void;
    /** Arena hook: open the GLB/glTF upload (the upload result becomes an `addGltf`). */
    onimportGlb?: () => void;
    readonly?: boolean;
  }
  let { doc, selectedId = $bindable<string | null>(null), onchange, onimportGlb, readonly = false }: Props = $props();

  interface Row {
    id: string;
    depth: number;
    node: Scene3dNode;
    hasChildren: boolean;
    collapsed: boolean;
  }

  let collapsed = $state<Record<string, boolean>>({});
  let renaming = $state<string | null>(null);
  let draft = $state('');
  let filter = $state('');

  const rows = $derived.by((): Row[] => {
    const out: Row[] = [];
    const parentOf = new Map<string, string>();
    for (const g of doc.groups) for (const c of g.children) parentOf.set(c, g.id);
    const seen = new Set<string>();
    const walk = (id: string, depth: number) => {
      if (seen.has(id)) return;
      const n = findNode(doc, id);
      if (!n || n.kind === 'light') return;
      seen.add(id);
      const kids = n.kind === 'group' ? n.node.children : [];
      const isCollapsed = Boolean(collapsed[id]);
      out.push({ id, depth, node: n, hasChildren: kids.length > 0, collapsed: isCollapsed });
      if (!isCollapsed) for (const c of kids) walk(c, depth + 1);
    };
    // Top level: groups first (doc order), then loose objects (doc order).
    for (const g of doc.groups) if (!parentOf.has(g.id)) walk(g.id, 0);
    for (const o of doc.objects) if (!parentOf.has(o.id)) walk(o.id, 0);
    // Orphans claimed by a missing group still show up (never lose a node).
    for (const o of doc.objects) if (!seen.has(o.id)) walk(o.id, 0);
    for (const g of doc.groups) if (!seen.has(g.id)) walk(g.id, 0);
    return out;
  });

  const lights = $derived(doc.lights);

  const q = $derived(filter.trim().toLowerCase());
  const visibleRows = $derived(q ? rows.filter((r) => (r.node.node.name ?? r.id).toLowerCase().includes(q)) : rows);
  const visibleLights = $derived(q ? lights.filter((l) => (l.name ?? l.id).toLowerCase().includes(q)) : lights);

  function iconFor(n: Scene3dNode): string {
    if (n.kind === 'group') return 'folder';
    if (n.kind === 'light') return 'zap';
    switch (n.node.type) {
      case 'gltf':
        return 'layers';
      case 'text':
        return 'note';
      case 'plane':
        return 'square';
      default:
        return 'box';
    }
  }

  function select(id: string): void {
    selectedId = id;
  }
  function toggle(id: string, e: MouseEvent): void {
    e.stopPropagation();
    collapsed = { ...collapsed, [id]: !collapsed[id] };
  }
  function startRename(id: string): void {
    if (readonly) return;
    const n = findNode(doc, id);
    if (!n) return;
    renaming = id;
    draft = n.node.name ?? id;
  }
  function commitRename(): void {
    if (renaming) {
      const next = rename(doc, renaming, draft);
      if (next !== doc) onchange(next);
    }
    renaming = null;
  }
  function onRenameKey(e: KeyboardEvent): void {
    if (e.key === 'Enter') {
      e.preventDefault();
      commitRename();
    } else if (e.key === 'Escape') {
      e.preventDefault();
      renaming = null;
    }
  }
  function toggleVisible(id: string, e: MouseEvent): void {
    e.stopPropagation();
    if (readonly) return;
    const n = findNode(doc, id);
    if (!n) return;
    onchange(setVisible(doc, id, n.node.visible === false));
  }
  function doDuplicate(id: string): void {
    const r = duplicate(doc, id);
    if (r) {
      onchange(r.doc);
      selectedId = r.id;
    }
  }
  function doDelete(id: string): void {
    onchange(remove(doc, id));
    if (selectedId === id) selectedId = null;
  }
  function onRowKey(e: KeyboardEvent, id: string): void {
    if (e.key === 'Enter' || e.key === ' ') {
      e.preventDefault();
      select(id);
    } else if (e.key === 'F2') {
      e.preventDefault();
      startRename(id);
    } else if (e.key === 'ContextMenu') {
      rowMenu(e, id);
    }
  }

  function rowMenu(e: MouseEvent | KeyboardEvent, id: string): void {
    select(id);
    const n = findNode(doc, id);
    if (!n) return;
    const items: MenuItem[] = [
      { label: 'Rename', icon: 'edit', disabled: readonly, action: () => startRename(id) },
      { label: 'Duplicate', icon: 'copy', disabled: readonly, action: () => doDuplicate(id) },
      { separator: true },
      { label: 'Move up', icon: 'arrowUp', disabled: readonly, action: () => onchange(reorder(doc, id, -1)) },
      { label: 'Move down', icon: 'arrowDown', disabled: readonly, action: () => onchange(reorder(doc, id, 1)) },
    ];
    if (n.kind !== 'light') {
      items.push({ separator: true });
      const parent = parentGroup(doc, id);
      const targets = doc.groups.filter((g) => g.id !== id && g.id !== parent?.id);
      for (const g of targets) {
        items.push({ label: `Move to ${g.name}`, icon: 'folder', disabled: readonly, action: () => onchange(moveToGroup(doc, id, g.id)) });
      }
      if (parent) {
        items.push({ label: 'Move to top level', icon: 'arrowUp', disabled: readonly, action: () => onchange(moveToGroup(doc, id, null)) });
      }
      items.push({
        label: 'Group this',
        icon: 'folder',
        disabled: readonly,
        action: () => {
          const r = addGroup(doc, 'Group', [id]);
          onchange(r.doc);
          selectedId = r.id;
        },
      });
    }
    items.push({ separator: true });
    items.push({
      label: n.kind === 'group' ? 'Delete group + children' : 'Delete',
      icon: 'trash',
      danger: true,
      disabled: readonly,
      action: () => doDelete(id),
    });
    ctxMenu.show(e, items);
  }

  const PRIMITIVE_LABEL: Record<PrimitiveType, string> = {
    box: 'Box',
    sphere: 'Sphere',
    cylinder: 'Cylinder',
    cone: 'Cone',
    torus: 'Torus',
    plane: 'Plane',
    text: 'Text',
  };
  const LIGHT_LABEL: Record<LightType, string> = {
    directional: 'Directional (sun)',
    ambient: 'Ambient',
    point: 'Point',
    spot: 'Spot',
    hemisphere: 'Hemisphere (sky)',
  };

  function addMenu(e: MouseEvent | KeyboardEvent): void {
    // New nodes land inside the selected group (or the selection's group) so
    // "select Props → Add → Box" does what a studio user expects.
    const sel = findNode(doc, selectedId);
    const groupId = sel?.kind === 'group' ? sel.node.id : selectedId ? parentGroup(doc, selectedId)?.id ?? null : null;
    const items: MenuItem[] = [];
    for (const t of PRIMITIVE_TYPES) {
      items.push({
        label: PRIMITIVE_LABEL[t],
        icon: t === 'text' ? 'note' : t === 'plane' ? 'square' : 'box',
        action: () => {
          const r = addPrimitive(doc, t, { groupId });
          onchange(r.doc);
          selectedId = r.id;
        },
      });
    }
    items.push({ separator: true });
    for (const t of LIGHT_TYPES) {
      items.push({
        label: `${LIGHT_LABEL[t]} light`,
        icon: 'zap',
        action: () => {
          const r = addLight(doc, t);
          onchange(r.doc);
          selectedId = r.id;
        },
      });
    }
    items.push({ separator: true });
    items.push({
      label: 'Group',
      icon: 'folder',
      action: () => {
        const r = addGroup(doc, 'Group', []);
        onchange(r.doc);
        selectedId = r.id;
        startRename(r.id);
      },
    });
    if (onimportGlb) {
      items.push({ label: 'Import GLB / glTF…', icon: 'arrowUp', action: () => onimportGlb?.() });
    }
    ctxMenu.show(e, items);
  }
</script>

<div class="s3d-hier">
  <div class="s3d-hier-head">
    <span class="s3d-hier-title">Hierarchy</span>
    <input class="s3d-hier-filter" placeholder="Filter…" bind:value={filter} aria-label="Filter hierarchy" />
    {#if !readonly}
      <button class="s3d-icon-btn" title="Add object, light or group" aria-label="Add" onclick={addMenu}>
        <Icon name="plus" size={14} />
      </button>
    {/if}
  </div>

  <div class="s3d-hier-body" role="tree" aria-label="Scene hierarchy">
    {#if !visibleRows.length && !visibleLights.length}
      <div class="s3d-hier-empty">
        {#if q}
          Nothing matches “{filter}”.
        {:else}
          Empty scene. Use <strong>+</strong> to add a primitive, or ask the assistant.
        {/if}
      </div>
    {/if}

    {#each visibleRows as r (r.id)}
      {@const hidden = r.node.node.visible === false}
      {@const dimmed = !isEffectivelyVisible(doc, r.id)}
      <div
        class="s3d-row"
        class:selected={selectedId === r.id}
        class:dimmed
        role="treeitem"
        tabindex="0"
        aria-selected={selectedId === r.id}
        aria-expanded={r.node.kind === 'group' ? !r.collapsed : undefined}
        aria-level={r.depth + 1}
        style="padding-inline-start: {8 + r.depth * 14}px"
        onclick={() => select(r.id)}
        ondblclick={() => startRename(r.id)}
        oncontextmenu={(e) => rowMenu(e, r.id)}
        onkeydown={(e) => onRowKey(e, r.id)}
      >
        {#if r.node.kind === 'group'}
          <button class="s3d-disclose" tabindex="-1" aria-label={r.collapsed ? 'Expand' : 'Collapse'} onclick={(e) => toggle(r.id, e)}>
            <Icon name={r.collapsed ? 'chevronRight' : 'chevronDown'} size={12} />
          </button>
        {:else}
          <span class="s3d-disclose spacer"></span>
        {/if}
        <span class="s3d-row-icon"><Icon name={iconFor(r.node)} size={13} /></span>
        {#if renaming === r.id}
          <!-- svelte-ignore a11y_autofocus -->
          <input
            class="s3d-rename"
            bind:value={draft}
            autofocus
            onblur={commitRename}
            onkeydown={onRenameKey}
            onclick={(e) => e.stopPropagation()}
            ondblclick={(e) => e.stopPropagation()}
          />
        {:else}
          <span class="s3d-row-name" title={r.node.node.name}>{r.node.node.name}</span>
          {#if r.node.kind === 'group'}
            <span class="s3d-row-count">{r.node.node.children.length}</span>
          {:else if r.node.kind === 'object' && r.node.node.type === 'gltf'}
            <span class="s3d-row-count">glb</span>
          {/if}
        {/if}
        <button
          class="s3d-icon-btn s3d-eye"
          class:off={hidden}
          tabindex="-1"
          title={hidden ? 'Show' : 'Hide'}
          aria-label={hidden ? 'Show' : 'Hide'}
          disabled={readonly}
          onclick={(e) => toggleVisible(r.id, e)}
        >
          <Icon name={hidden ? 'eyeOff' : 'eye'} size={13} />
        </button>
        <button
          class="s3d-icon-btn s3d-more"
          tabindex="-1"
          title="More"
          aria-label="More actions"
          onclick={(e) => {
            e.stopPropagation();
            rowMenu(e, r.id);
          }}
        >
          <Icon name="dot" size={13} />
        </button>
      </div>
    {/each}

    {#if visibleLights.length}
      <div class="s3d-section">Lights</div>
      {#each visibleLights as l (l.id)}
        {@const hidden = l.visible === false}
        <div
          class="s3d-row"
          class:selected={selectedId === l.id}
          class:dimmed={hidden}
          role="treeitem"
          tabindex="0"
          aria-selected={selectedId === l.id}
          aria-level={1}
          style="padding-inline-start: 8px"
          onclick={() => select(l.id)}
          ondblclick={() => startRename(l.id)}
          oncontextmenu={(e) => rowMenu(e, l.id)}
          onkeydown={(e) => onRowKey(e, l.id)}
        >
          <span class="s3d-disclose spacer"></span>
          <span class="s3d-row-icon light"><Icon name="zap" size={13} /></span>
          {#if renaming === l.id}
            <!-- svelte-ignore a11y_autofocus -->
            <input
              class="s3d-rename"
              bind:value={draft}
              autofocus
              onblur={commitRename}
              onkeydown={onRenameKey}
              onclick={(e) => e.stopPropagation()}
              ondblclick={(e) => e.stopPropagation()}
            />
          {:else}
            <span class="s3d-row-name" title={l.name ?? l.id}>{l.name ?? l.id}</span>
            <span class="s3d-row-count">{l.type}</span>
          {/if}
          <button
            class="s3d-icon-btn s3d-eye"
            class:off={hidden}
            tabindex="-1"
            title={hidden ? 'Turn on' : 'Turn off'}
            aria-label={hidden ? 'Turn on' : 'Turn off'}
            disabled={readonly}
            onclick={(e) => toggleVisible(l.id, e)}
          >
            <Icon name={hidden ? 'eyeOff' : 'eye'} size={13} />
          </button>
          <button
            class="s3d-icon-btn s3d-more"
            tabindex="-1"
            title="More"
            aria-label="More actions"
            onclick={(e) => {
              e.stopPropagation();
              rowMenu(e, l.id);
            }}
          >
            <Icon name="dot" size={13} />
          </button>
        </div>
      {/each}
    {/if}
  </div>
</div>

<style>
  .s3d-hier {
    display: flex;
    flex-direction: column;
    min-height: 0;
    height: 100%;
    font-size: 12px;
    color: var(--text);
  }
  .s3d-hier-head {
    display: flex;
    align-items: center;
    gap: 6px;
    padding: 8px 8px 6px;
    border-bottom: 1px solid var(--border);
    flex-shrink: 0;
  }
  .s3d-hier-title {
    font-size: 10.5px;
    font-weight: 600;
    text-transform: uppercase;
    letter-spacing: 0.04em;
    color: var(--text-dim);
  }
  .s3d-hier-filter {
    flex: 1 1 auto;
    min-width: 0;
    padding: 3px 6px;
    border: 1px solid var(--border);
    border-radius: var(--radius-s, 5px);
    background: var(--bg);
    color: var(--text);
    font-size: 11px;
  }
  .s3d-hier-body {
    flex: 1 1 auto;
    overflow-y: auto;
    padding: 4px 0 8px;
  }
  .s3d-hier-empty {
    padding: 12px 10px;
    color: var(--text-dim);
    line-height: 1.5;
  }
  .s3d-section {
    padding: 8px 8px 3px;
    font-size: 10px;
    font-weight: 600;
    text-transform: uppercase;
    letter-spacing: 0.04em;
    color: var(--text-dim);
  }
  .s3d-row {
    display: flex;
    align-items: center;
    gap: 4px;
    height: 26px;
    padding-inline-end: 4px;
    cursor: default;
    border-radius: var(--radius-s, 5px);
    margin: 0 4px;
    outline: none;
  }
  .s3d-row:hover {
    background: var(--surface-2);
  }
  .s3d-row:focus-visible {
    box-shadow: inset 0 0 0 1px var(--accent);
  }
  .s3d-row.selected {
    background: color-mix(in srgb, var(--accent) 18%, transparent);
  }
  .s3d-row.dimmed .s3d-row-name,
  .s3d-row.dimmed .s3d-row-icon {
    opacity: 0.45;
  }
  .s3d-disclose {
    appearance: none;
    border: 0;
    background: transparent;
    color: var(--text-dim);
    width: 16px;
    height: 16px;
    display: inline-flex;
    align-items: center;
    justify-content: center;
    padding: 0;
    cursor: pointer;
    flex-shrink: 0;
  }
  .s3d-disclose.spacer {
    pointer-events: none;
  }
  .s3d-row-icon {
    display: inline-flex;
    color: var(--text-dim);
    flex-shrink: 0;
  }
  .s3d-row-icon.light {
    color: var(--status-warn, #e0a000);
  }
  .s3d-row-name {
    flex: 1 1 auto;
    min-width: 0;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .s3d-row-count {
    font-size: 10px;
    color: var(--text-dim);
    font-variant-numeric: tabular-nums;
    flex-shrink: 0;
  }
  .s3d-rename {
    flex: 1 1 auto;
    min-width: 0;
    padding: 2px 4px;
    border: 1px solid var(--accent);
    border-radius: 3px;
    background: var(--bg);
    color: var(--text);
    font-size: 12px;
  }
  .s3d-icon-btn {
    appearance: none;
    border: 0;
    background: transparent;
    color: var(--text-dim);
    width: 22px;
    height: 22px;
    border-radius: var(--radius-s, 5px);
    display: inline-flex;
    align-items: center;
    justify-content: center;
    padding: 0;
    cursor: pointer;
    flex-shrink: 0;
  }
  .s3d-icon-btn:hover:not(:disabled) {
    background: var(--surface-2);
    color: var(--text);
  }
  .s3d-icon-btn:disabled {
    opacity: 0.4;
    cursor: default;
  }
  .s3d-eye,
  .s3d-more {
    opacity: 0;
  }
  .s3d-row:hover .s3d-eye,
  .s3d-row:hover .s3d-more,
  .s3d-row.selected .s3d-eye,
  .s3d-row:focus-within .s3d-eye,
  .s3d-eye.off {
    opacity: 1;
  }
  @media (hover: none) {
    .s3d-eye,
    .s3d-more {
      opacity: 1;
    }
  }
</style>
