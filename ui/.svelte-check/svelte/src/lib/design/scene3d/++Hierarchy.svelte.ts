///<reference types="svelte" />
;
import { rowMenu as rowMenuKeys } from '../../rowMenu';
// Scene hierarchy — the game-studio left panel while a scene3d artifact is open
// (the arena toggles it in place of the asset list). Groups collapse, rows
// select, double-click / F2 renames inline, the eye hides, the ⋯ / right-click
// menu duplicates, deletes, moves to a group or ungroups. The Add ▾ menu offers
// primitives, lights, a group and "Import GLB…" (the upload itself is the
// arena's — `onimportGlb`). Every edit is an `ops.ts` call → `onchange(newDoc)`.
import { tick } from 'svelte';
import { ctxMenu, type MenuItem } from '../../contextmenu.svelte';
import { focusOnMount } from '../../focusOnMount';
import Icon, { type IconName } from '../../components/Icon.svelte';
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
;

  interface Props {
    doc: Scene3dDoc;
    selectedId?: string | null;
    onchange: (doc: Scene3dDoc) => void;
    /** Arena hook: open the GLB/glTF upload (the upload result becomes an `addGltf`). */
    onimportGlb?: () => void;
    readonly?: boolean;
  };;

  interface Row {
    id: string;
    depth: number;
    node: Scene3dNode;
    hasChildren: boolean;
    collapsed: boolean;
  };function $$render() {

  
  
  
  
  
  
  
  
  
  
  
  
  

  let { doc, selectedId = $bindable<string | null>(null), onchange, onimportGlb, readonly = false }: Props = $props()/*Ωignore_startΩ*/;selectedId;/*Ωignore_endΩ*/;


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

  function iconFor(n: Scene3dNode): IconName {
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
  // ── Tree keyboard (WAI-ARIA tree view) ─────────────────────────────────────
  // One treeitem is in the Tab order (the selection, else the first row);
  // ↑/↓/Home/End move it, → expands a group or steps into it, ← collapses or
  // steps out to the parent group (mirrored under RTL). Selection follows
  // focus, like the viewport's own click-to-select. The eye / ⋯ / disclosure
  // buttons are pointer shortcuts — every one of their actions is also on the
  // keyboard (arrows, ContextMenu / ⇧F10 → the row menu, which has Hide/Show).
  let treeEl = $state<HTMLElement | null>(null);
  const order = $derived([...visibleRows.map((r) => r.id), ...visibleLights.map((l) => l.id)]);
  const tabStop = $derived(selectedId && order.includes(selectedId) ? selectedId : (order[0] ?? null));

  function focusItem(id: string | null | undefined): void {
    if (!id) return;
    select(id);
    void tick().then(() => treeEl?.querySelector<HTMLElement>(`[data-node-id="${CSS.escape(id)}"]`)?.focus());
  }
  function setCollapsed(id: string, value: boolean): void {
    collapsed = { ...collapsed, [id]: value };
  }
  function onRowKey(e: KeyboardEvent, id: string): void {
    if (e.target !== e.currentTarget) return; // the inline rename input owns its keys
    const i = order.indexOf(id);
    const row = visibleRows.find((r) => r.id === id);
    const rtl = treeEl ? getComputedStyle(treeEl).direction === 'rtl' : false;
    const key = rtl && e.key === 'ArrowRight' ? 'ArrowLeft' : rtl && e.key === 'ArrowLeft' ? 'ArrowRight' : e.key;
    if (key === 'ArrowDown' || key === 'ArrowUp' || key === 'Home' || key === 'End') {
      e.preventDefault();
      const next = key === 'Home' ? 0 : key === 'End' ? order.length - 1 : i + (key === 'ArrowDown' ? 1 : -1);
      focusItem(order[Math.max(0, Math.min(order.length - 1, next))]);
    } else if (key === 'ArrowRight') {
      e.preventDefault();
      if (row?.hasChildren && row.collapsed) setCollapsed(id, false);
      else if (row?.hasChildren) focusItem(order[i + 1]);
    } else if (key === 'ArrowLeft') {
      e.preventDefault();
      if (row?.hasChildren && !row.collapsed) setCollapsed(id, true);
      else focusItem(parentGroup(doc, id)?.id);
    } else if (e.key === 'Enter' || e.key === ' ') {
      e.preventDefault();
      select(id);
    } else if (e.key === 'F2') {
      e.preventDefault();
      startRename(id);
    } else if (e.key === 'ContextMenu' || (e.key === 'F10' && e.shiftKey)) {
      e.preventDefault();
      rowMenu(e, id);
    }
  }

  function rowMenu(e: MouseEvent | KeyboardEvent, id: string): void {
    select(id);
    const n = findNode(doc, id);
    if (!n) return;
    const hiddenNow = n.node.visible === false;
    const items: MenuItem[] = [
      { label: 'Rename', icon: 'edit', disabled: readonly, action: () => startRename(id) },
      {
        label: n.kind === 'light' ? (hiddenNow ? 'Turn on' : 'Turn off') : hiddenNow ? 'Show' : 'Hide',
        icon: 'eye',
        disabled: readonly,
        action: () => onchange(setVisible(doc, id, hiddenNow)),
      },
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
;
async () => {

 { svelteHTML.createElement("div", { "class":`s3d-hier`,});
   { svelteHTML.createElement("div", { "class":`s3d-hier-head`,});
     { svelteHTML.createElement("span", { "class":`s3d-hier-title`,});  }
     { svelteHTML.createElement("input", {          "dir":`ltr`,"class":`s3d-hier-filter`,"placeholder":`Filter objects…`,"bind:value":filter,"aria-label":`Filter objects`,});/*Ωignore_startΩ*/() => filter = __sveltets_2_any(null);/*Ωignore_endΩ*/}
    if(!readonly){
       { svelteHTML.createElement("button", {       "class":`s3d-icon-btn`,"title":`Add object, light or group`,"aria-label":`Add object, light or group`,"onclick":addMenu,});
         { const $$_nocI3C = __sveltets_2_ensureComponent(Icon); new $$_nocI3C({ target: __sveltets_2_any(), props: {    "name":`plus`,"size":14,}});}
       }
    }
   }

   { const $$_div1 = svelteHTML.createElement("div", {      "class":`s3d-hier-body`,"role":`tree`,"aria-label":`Scene hierarchy`,});treeEl = $$_div1;
    if(!visibleRows.length && !visibleLights.length){
       { svelteHTML.createElement("div", { "class":`s3d-hier-empty`,});
        if(q){  filter; }else{    { svelteHTML.createElement("strong", {});  }        }
       }
    }

       for(let r of __sveltets_2_ensureArray(visibleRows)){r.id;
      const hidden = r.node.node.visible === false;
      const dimmed = !isEffectivelyVisible(doc, r.id);
       {const $$action_0 = __sveltets_2_ensureAction(rowMenuKeys(svelteHTML.mapElementTag('div')));{ svelteHTML.createElement("div", __sveltets_2_union($$action_0), {                            "class":`s3d-row`,"role":`treeitem`,...__sveltets_2_empty({"data-node-id":r.id}),"tabindex":tabStop === r.id ? 0 : -1,"aria-selected":selectedId === r.id,"aria-expanded":r.hasChildren ? !r.collapsed : undefined,"aria-level":r.depth + 1,"style":`padding-inline-start: ${8 + r.depth * 14}px`,"onclick":() => select(r.id),"ondblclick":() => startRename(r.id),"oncontextmenu":(e) => rowMenu(e, r.id),"onkeydown":(e) => onRowKey(e, r.id),});selectedId === r.id;dimmed;
        if(r.node.kind === 'group'){
          
           { svelteHTML.createElement("button", {           "class":`s3d-disclose`,"tabindex":-1,"aria-hidden":`true`,"aria-label":r.collapsed ? 'Expand (→)' : 'Collapse (←)',"title":r.collapsed ? 'Expand (→)' : 'Collapse (←)',"onclick":(e) => toggle(r.id, e),});
             { const $$_nocI4C = __sveltets_2_ensureComponent(Icon); new $$_nocI4C({ target: __sveltets_2_any(), props: {    "name":r.collapsed ? 'chevronRight' : 'chevronDown',"size":12,}});}
           }
        }else{
           { svelteHTML.createElement("span", { "class":`s3d-disclose spacer`,}); }
        }
         { svelteHTML.createElement("span", { "class":`s3d-row-icon`,}); { const $$_nocI4C = __sveltets_2_ensureComponent(Icon); new $$_nocI4C({ target: __sveltets_2_any(), props: {    "name":iconFor(r.node),"size":13,}});} }
        if(renaming === r.id){
           {const $$action_0 = __sveltets_2_ensureAction(focusOnMount(svelteHTML.mapElementTag('input'),({ select: true })));{ svelteHTML.createElement("input", __sveltets_2_union($$action_0), {                  "dir":`auto`,"class":`s3d-rename`,"aria-label":`Name`,"bind:value":draft,"onblur":commitRename,"onkeydown":onRenameKey,"onclick":(e) => e.stopPropagation(),"ondblclick":(e) => e.stopPropagation(),});/*Ωignore_startΩ*/() => draft = __sveltets_2_any(null);/*Ωignore_endΩ*/}}
        }else{
           { svelteHTML.createElement("span", {   "class":`s3d-row-name`,"title":r.node.node.name,});r.node.node.name; }
          if(r.node.kind === 'group'){
             { svelteHTML.createElement("span", { "class":`s3d-row-count`,});r.node.node.children.length; }
          } else if (r.node.kind === 'object' && r.node.node.type === 'gltf'){
             { svelteHTML.createElement("span", { "class":`s3d-row-count`,});  }
          }
        }
         { svelteHTML.createElement("button", {              "class":`s3d-icon-btn s3d-eye`,"tabindex":-1,"title":hidden ? 'Show' : 'Hide',"aria-label":hidden ? 'Show' : 'Hide',"disabled":readonly,"onclick":(e) => toggleVisible(r.id, e),});hidden;
           { const $$_nocI4C = __sveltets_2_ensureComponent(Icon); new $$_nocI4C({ target: __sveltets_2_any(), props: {    "name":hidden ? 'eyeOff' : 'eye',"size":13,}});}
         }
         { svelteHTML.createElement("button", {           "class":`s3d-icon-btn s3d-more`,"tabindex":-1,"title":`More actions`,"aria-label":`More actions`,"onclick":(e) => {
            e.stopPropagation();
            rowMenu(e, r.id);
          },});
           { const $$_nocI4C = __sveltets_2_ensureComponent(Icon); new $$_nocI4C({ target: __sveltets_2_any(), props: {    "name":`dot`,"size":13,}});}
         }
       }}
    }

    if(visibleLights.length){
       { svelteHTML.createElement("div", {   "class":`s3d-section`,"role":`presentation`,});  }
         for(let l of __sveltets_2_ensureArray(visibleLights)){l.id;
        const hidden = l.visible === false;
         {const $$action_0 = __sveltets_2_ensureAction(rowMenuKeys(svelteHTML.mapElementTag('div')));{ svelteHTML.createElement("div", __sveltets_2_union($$action_0), {                          "class":`s3d-row`,"role":`treeitem`,...__sveltets_2_empty({"data-node-id":l.id}),"tabindex":tabStop === l.id ? 0 : -1,"aria-selected":selectedId === l.id,"aria-level":1,"style":`padding-inline-start: 8px`,"onclick":() => select(l.id),"ondblclick":() => startRename(l.id),"oncontextmenu":(e) => rowMenu(e, l.id),"onkeydown":(e) => onRowKey(e, l.id),});selectedId === l.id;hidden;
           { svelteHTML.createElement("span", { "class":`s3d-disclose spacer`,}); }
           { svelteHTML.createElement("span", { "class":`s3d-row-icon light`,}); { const $$_nocI4C = __sveltets_2_ensureComponent(Icon); new $$_nocI4C({ target: __sveltets_2_any(), props: {    "name":`zap`,"size":13,}});} }
          if(renaming === l.id){
             {const $$action_0 = __sveltets_2_ensureAction(focusOnMount(svelteHTML.mapElementTag('input'),({ select: true })));{ svelteHTML.createElement("input", __sveltets_2_union($$action_0), {                  "dir":`auto`,"class":`s3d-rename`,"aria-label":`Name`,"bind:value":draft,"onblur":commitRename,"onkeydown":onRenameKey,"onclick":(e) => e.stopPropagation(),"ondblclick":(e) => e.stopPropagation(),});/*Ωignore_startΩ*/() => draft = __sveltets_2_any(null);/*Ωignore_endΩ*/}}
          }else{
             { svelteHTML.createElement("span", {   "class":`s3d-row-name`,"title":l.name ?? l.id,});l.name ?? l.id; }
             { svelteHTML.createElement("span", { "class":`s3d-row-count`,});l.type; }
          }
           { svelteHTML.createElement("button", {              "class":`s3d-icon-btn s3d-eye`,"tabindex":-1,"title":hidden ? 'Turn on' : 'Turn off',"aria-label":hidden ? 'Turn on' : 'Turn off',"disabled":readonly,"onclick":(e) => toggleVisible(l.id, e),});hidden;
             { const $$_nocI4C = __sveltets_2_ensureComponent(Icon); new $$_nocI4C({ target: __sveltets_2_any(), props: {    "name":hidden ? 'eyeOff' : 'eye',"size":13,}});}
           }
           { svelteHTML.createElement("button", {           "class":`s3d-icon-btn s3d-more`,"tabindex":-1,"title":`More actions`,"aria-label":`More actions`,"onclick":(e) => {
              e.stopPropagation();
              rowMenu(e, l.id);
            },});
             { const $$_nocI4C = __sveltets_2_ensureComponent(Icon); new $$_nocI4C({ target: __sveltets_2_any(), props: {    "name":`dot`,"size":13,}});}
           }
         }}
      }
    }
   }
 }


};
return { props: {} as any as Props, exports: {}, bindings: __sveltets_$$bindings('selectedId'), slots: {}, events: {} }}
const Hierarchy__SvelteComponent_ = __sveltets_2_fn_component($$render());
/*Ωignore_startΩ*/type Hierarchy__SvelteComponent_ = ReturnType<typeof Hierarchy__SvelteComponent_>;
/*Ωignore_endΩ*/export default Hierarchy__SvelteComponent_;