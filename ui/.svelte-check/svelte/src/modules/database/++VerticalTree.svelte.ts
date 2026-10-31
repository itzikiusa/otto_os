///<reference types="svelte" />
;
import { rowMenu } from '../../lib/rowMenu';
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
//
// Keyboard (VerticalView owns the record's role="tree" and its roving
// tabindex / arrow navigation): every row is a `treeitem` (aria-level, and
// aria-expanded on containers). On a focused row Enter or F2 starts the same
// edit as a double-click (a container toggles instead), →/← open/close a
// container (mirrored in RTL), and ⇧F10 / the ContextMenu key open the same field menu as a
// right-click — which the row's ⋯ button (shown on hover/focus) also opens.
//
// ⌘F (json-find.ts): every row carries `data-jpath` + `data-jcol`; text the
// find model leaves out (chevrons, summaries, array-index labels, rename /
// unset marks, "more" buttons) is `data-find-skip`. A find reveal (`reveal`, per record)
// forces paths open, grows "show more" slices and unclips strings.
import Self from './VerticalTree.svelte';
import ValueEditor from './ValueEditor.svelte';
import { bsonScalar } from '../../lib/jsonTree/bson';
import type { DbEngine } from '../../lib/api/types';
import { typedRaw, type EditFlow } from './EditFlow.svelte';
import type { FieldCtx, TypedValue } from './edit-types';
import { CHUNK, entriesOf, setOverride, stickyKey, valueKind, type ExpansionState } from '../../lib/jsonTree/expansion-plan';
import { cellStr } from './results-format';
import { searchableLabel, STR_MAX, type RevealState } from '../../lib/jsonTree/json-find';
;

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
    /** The field menu (right-click, ⋯, ⇧F10). Omitted → no menu (mini views). */
    onfieldmenu?: (e: MouseEvent | KeyboardEvent, ctx: FieldCtx) => void;
    /** This record's find-reveal overlay. */
    reveal?: RevealState;
    /** A pending `$set` on a path that doesn't exist yet — not in the find
     *  model, so its row is `data-find-skip`. */
    phantom?: boolean;
  };function $$render() {
  const moreButton/*Ωignore_positionΩ*/ = ()/*Ωignore_startΩ*/: ReturnType<import('svelte').Snippet>/*Ωignore_endΩ*/ => { async ()/*Ωignore_positionΩ*/ => {
  if(onfieldmenu){
     { svelteHTML.createElement("button", {              "class":`vrow-more`,"type":`button`,"tabindex":-1,"aria-label":`Field actions for ${path}`,"title":`Field actions for ${path}`,"onclick":(e) => onfieldmenu?.(e, ctx()),...__sveltets_2_empty({"data-find-skip":true}),});  }
  }
};return __sveltets_2_any(0)};
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  

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
    reveal,
    phantom = false,
  }: Props = $props();

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

  // An override-opened path the planner never reached (a child past its
  // parent's first CHUNK, drawn by "show more") is open too — the plan only
  // queues the first slice.
  const open = $derived(
    plan.has(path) || expansion.overrides.get(stickyKey(path)) === true || !!reveal?.opens.has(path),
  );
  let shownLocal = $state(CHUNK);
  const shown = $derived(Math.max(shownLocal, reveal?.shown.get(path) ?? 0));
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
  let strOpenLocal = $state(false);
  const strOpen = $derived(strOpenLocal || !!reveal?.strs.has(path));
  function toggleStr(): void {
    strOpenLocal = !strOpen;
    reveal?.strs.delete(path);
  }
  const keySkip = $derived(!searchableLabel(label));
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
    const next = !open;
    reveal?.opens.delete(path);
    setOverride(expansion, path, next);
  }

  /** The row's accessible name: "field: value" (what a sighted user reads). */
  const rowName = $derived.by(() => {
    if (!asLeaf) return `${label}: ${summary}`;
    if (pend === 'unset') return `${label}: ${text} (pending: unset)`;
    if (pendTyped !== null) return `${label}: ${pendingText(pendTyped)} (pending)`;
    if (value === null || value === undefined) return `${label}: null`;
    if (bson !== null) return `${label}: ${bson}`;
    if (container) return `${label}: ${summary}`;
    return `${label}: ${strLong && !strOpen ? text.slice(0, STR_MAX) + '…' : text}`;
  });

  /** Keys on the focused row itself (never from the inline editor inside it).
   *  Arrow ↑/↓/Home/End and ← to the parent are the tree's (VerticalView). */
  function onRowKey(e: KeyboardEvent): void {
    if (e.target !== e.currentTarget || e.metaKey || e.ctrlKey || e.altKey) return;
    if ((e.key === 'F10' && e.shiftKey) || e.key === 'ContextMenu') {
      onfieldmenu?.(e, ctx());
      return;
    }
    if (e.shiftKey) return;
    if (e.key === 'Enter' || e.key === 'F2') {
      if (!asLeaf) {
        if (e.key === 'Enter') {
          e.preventDefault();
          toggle();
        }
      } else if (canEdit) {
        e.preventDefault();
        beginEdit();
      }
    } else if (!asLeaf && (e.key === 'ArrowRight' || e.key === 'ArrowLeft')) {
      // → opens / ← closes (mirrored in RTL); otherwise the tree moves focus.
      const rtl = getComputedStyle(e.currentTarget as HTMLElement).direction === 'rtl';
      const opening = (e.key === 'ArrowRight') !== rtl;
      if (opening !== open) {
        e.preventDefault();
        toggle();
      }
    }
  }

  // An inline edit that ends (saved, canceled, or the row re-rendered) hands
  // focus back to its row when nothing else took it — the keyboard user stays
  // in the tree instead of landing on <body>.
  let rowEl = $state<HTMLElement | null>(null);
  let wasEditing = false;
  $effect(() => {
    if (editing) {
      wasEditing = true;
      return;
    }
    if (!wasEditing) return;
    wasEditing = false;
    const el = rowEl;
    queueMicrotask(() => {
      const a = document.activeElement;
      if (el?.isConnected && (!a || a === document.body)) el.focus();
    });
  });
;
async () => {



if(asLeaf){
   {const $$action_0 = __sveltets_2_ensureAction(rowMenu(svelteHTML.mapElementTag('div')));{ const $$_div0 = svelteHTML.createElement("div", __sveltets_2_union($$action_0), {                                 "class":`vrow`,"role":`treeitem`,"aria-level":depth + 1,"aria-selected":`false`,"aria-label":rowName,"tabindex":-1,"style":`--depth:${depth}`,...__sveltets_2_empty({"data-jpath":path}),...__sveltets_2_empty({"data-jcol":colIdx}),...__sveltets_2_empty({"data-find-skip":phantom || undefined}),"title":dirty ? 'Pending change — Review & apply (bar below) writes it' : canEdit ? 'Double-click or press Enter to edit' : undefined,"ondblclick":beginEdit,"onkeydown":onRowKey,"oncontextmenu":(e) => onfieldmenu?.(e, ctx()),});dirty;canEdit;rowEl = $$_div0;
     { svelteHTML.createElement("span", {     "class":`vk mono`,"title":path,...__sveltets_2_empty({"data-find-skip":keySkip || undefined}),});label;if(renamedTo !== null){ { svelteHTML.createElement("span", {  "class":`vk-ren`,...__sveltets_2_empty({"data-find-skip":true}),});  renamedTo; }} }
    if(editing){
       { svelteHTML.createElement("span", { "class":`vv mono edit`,}); { const $$_rotidEeulaV2C = __sveltets_2_ensureComponent(ValueEditor); new $$_rotidEeulaV2C({ target: __sveltets_2_any(), props: {         "kind":editKind,"raw":editRaw,engine,"onsave":(tv) => flow.parkPath(rowIdx, colIdx, path, tv),"oncancel":() => flow.cancelPathEdit(),}});} }
    } else if (pend === 'unset'){
       { svelteHTML.createElement("span", { "class":`vv mono pend`,}); { svelteHTML.createElement("s", {});text; }  { svelteHTML.createElement("em", {...__sveltets_2_empty({"data-find-skip":true}),});  } }
    } else if (pendTyped !== null){
       { svelteHTML.createElement("span", {  "class":`vv mono pend`,});pendTyped.kind === 'null';pendingText(pendTyped); }
    } else if (value === null || value === undefined){
       { svelteHTML.createElement("span", { "class":`vv mono null-glyph`,});  }
    } else if (bson !== null){
       { svelteHTML.createElement("span", { "class":`vv mono bson`,});bson; }
    } else if (strLong){
       { svelteHTML.createElement("span", { "class":`vv mono tree`,});strOpen ? text : text.slice(0, STR_MAX) + '…'; { svelteHTML.createElement("button", {      "class":`vmore inline`,"type":`button`,"onclick":toggleStr,...__sveltets_2_empty({"data-find-skip":true}),});strOpen ? 'less' : `${text.length - STR_MAX} more chars`; } }
    } else if (container){
       { svelteHTML.createElement("span", { "class":`vv mono dim`,});summary; }
    }else{
       { svelteHTML.createElement("span", { "class":`vv mono`,});text; }
    }
    ;__sveltets_2_ensureSnippet(moreButton());
   }}
}else{
   {const $$action_0 = __sveltets_2_ensureAction(rowMenu(svelteHTML.mapElementTag('div')));{ const $$_div0 = svelteHTML.createElement("div", __sveltets_2_union($$action_0), {                             "class":`vrow ctr`,"role":`treeitem`,"aria-level":depth + 1,"aria-expanded":open,"aria-selected":`false`,"aria-label":rowName,"tabindex":-1,"style":`--depth:${depth}`,...__sveltets_2_empty({"data-jpath":path}),...__sveltets_2_empty({"data-jcol":colIdx}),"onkeydown":onRowKey,"oncontextmenu":(e) => onfieldmenu?.(e, ctx()),});dirtyInside;dirty;rowEl = $$_div0;
     { svelteHTML.createElement("span", {     "class":`vk mono`,"title":path,...__sveltets_2_empty({"data-find-skip":keySkip || undefined}),});label;if(renamedTo !== null){ { svelteHTML.createElement("span", {  "class":`vk-ren`,...__sveltets_2_empty({"data-find-skip":true}),});  renamedTo; }} }
     { svelteHTML.createElement("span", { "class":`vv mono tree`,});
      
       { svelteHTML.createElement("button", {            "class":`vsum`,"type":`button`,"tabindex":-1,"aria-expanded":open,"onclick":toggle,"title":open ? 'Collapse' : 'Expand',...__sveltets_2_empty({"data-find-skip":true}),});
         { svelteHTML.createElement("span", {   "class":`chev`,"aria-hidden":`true`,});open ? '▾' : '▸'; } { svelteHTML.createElement("span", { });open;summary; }
       }
     }
    ;__sveltets_2_ensureSnippet(moreButton());
   }}
  if(open){
     { svelteHTML.createElement("div", {       "class":`vnest`,"role":`group`,"aria-label":label,"style":`--depth:${depth + 1}`,});
         for(let [k, v] of __sveltets_2_ensureArray(visible)){k;
         { const $$_fleS1C = __sveltets_2_ensureComponent(Self); new $$_fleS1C({ target: __sveltets_2_any(), props: {                 "value":v,"path":`${path}.${k}`,"depth":depth + 1,"label":k,expansion,plan,editable,rowIdx,colIdx,flow,engine,onfieldmenu,reveal,}});}
      }
         for(let [k] of __sveltets_2_ensureArray(phantoms)){k;
          { const $$_fleS1C = __sveltets_2_ensureComponent(Self); new $$_fleS1C({ target: __sveltets_2_any(), props: {                "value":undefined,"path":`${path}.${k}`,"depth":depth + 1,"label":k,expansion,plan,editable,rowIdx,colIdx,flow,engine,onfieldmenu,"phantom":true,}});}
      }
      if(hiddenCount > 0){
         { svelteHTML.createElement("button", {      "class":`vmore`,"type":`button`,"onclick":() => (shownLocal = shown + CHUNK),...__sveltets_2_empty({"data-find-skip":true}),});
           Math.min(CHUNK, hiddenCount);   hiddenCount; 
         }
      }
     }
  }
}


};
return { props: {} as any as Props, exports: {}, bindings: __sveltets_$$bindings(''), slots: {}, events: {} }}
const VerticalTree__SvelteComponent_ = __sveltets_2_fn_component($$render());
/*Ωignore_startΩ*/type VerticalTree__SvelteComponent_ = ReturnType<typeof VerticalTree__SvelteComponent_>;
/*Ωignore_endΩ*/export default VerticalTree__SvelteComponent_;