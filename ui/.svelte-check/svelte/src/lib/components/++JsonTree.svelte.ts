///<reference types="svelte" />
;// Collapsible, LAZILY-rendered JSON tree.
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
// below, local open state — the cell viewer, AWS views, dashboards and the
// workbench JSON preview. (The API client renders its own
// modules/api/JsonTree.svelte.) The pure helpers live beside it in
// lib/jsonTree/, so this shared component imports nothing from modules/.
// Controlled (`plan` + `expansion` given, the JSON result view): a node is
// open iff its dotted `path` is in the owner's plan (a node budget with
// sticky per-path overrides, see expansion-plan.ts); toggling records an
// override on the shared state and the owner re-plans every record.
//
// ⌘F (json-find.ts): every line carries `data-jpath`; text the find model
// leaves out (chevrons, separators, summaries, array-index labels, "more"
// buttons) is `data-find-skip`. A find reveal (`reveal`, per record) forces
// paths open, grows "show more" slices and unclips strings on top of the plan.

import { tick } from 'svelte';
import Self from './JsonTree.svelte';
import { bsonScalar } from '../jsonTree/bson';
import { setOverride, stickyKey, type ExpansionState } from '../jsonTree/expansion-plan';
import { searchableLabel, STR_MAX, type RevealState } from '../jsonTree/json-find';
;

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
  };function $$render() {

  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  

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

  let kidsEl = $state<HTMLDivElement>();
  /** Reveal the next chunk. The button stays while more is hidden (focus
   *  stays on it); when the last chunk unmounts it, focus would fall to
   *  <body>, so it moves to the last revealed row — where the button was. */
  async function showMore(): Promise<void> {
    shownLocal = shown + CHUNK;
    await tick();
    if (hiddenCount > 0 || !kidsEl) return;
    const rows = kidsEl.querySelectorAll<HTMLElement>(':scope > .node > .line, :scope > .line');
    const last = rows[rows.length - 1];
    if (!last) return;
    if (last.tabIndex < 0 && !(last instanceof HTMLButtonElement)) last.tabIndex = -1;
    last.focus();
  }

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
;
async () => {

if(isContainer){
   { svelteHTML.createElement("div", {  "class":`node`,});depth === 0;
    if(size === 0){
       { svelteHTML.createElement("div", {   "class":`line`,...__sveltets_2_empty({"data-jpath":path}),});
        if(label !== null){ { svelteHTML.createElement("span", {   "class":`k`,...__sveltets_2_empty({"data-find-skip":keySkip || undefined}),});label; } { svelteHTML.createElement("span", {  "class":`sep`,...__sveltets_2_empty({"data-find-skip":true}),});  }}
         { svelteHTML.createElement("span", { "class":`empty`,});summary; }
       }
    }else{
       { svelteHTML.createElement("button", {             "class":`line toggle`,"type":`button`,"aria-expanded":open,"onclick":toggle,"title":open ? 'Collapse' : 'Expand',...__sveltets_2_empty({"data-jpath":path}),});
         { svelteHTML.createElement("span", {    "class":`chev`,"aria-hidden":`true`,...__sveltets_2_empty({"data-find-skip":true}),});open ? '▾' : '▸'; }
        if(label !== null){ { svelteHTML.createElement("span", {   "class":`k`,...__sveltets_2_empty({"data-find-skip":keySkip || undefined}),});label; } { svelteHTML.createElement("span", {  "class":`sep`,...__sveltets_2_empty({"data-find-skip":true}),});  }}
         { svelteHTML.createElement("span", {   "class":`sum`,...__sveltets_2_empty({"data-find-skip":true}),});open;summary; }
       }
      if(open){
         { const $$_div1 = svelteHTML.createElement("div", {  "class":`kids`,});kidsEl = $$_div1;
             for(let [k, v] of __sveltets_2_ensureArray(visible)){k;
             { const $$_fleS2C = __sveltets_2_ensureComponent(Self); new $$_fleS2C({ target: __sveltets_2_any(), props: {           "value":v,"label":k,"depth":depth + 1,"path":path ? `${path}.${k}` : k,plan,expansion,reveal,}});}
          }
          if(hiddenCount > 0){
             { svelteHTML.createElement("button", {      "class":`more`,"type":`button`,"onclick":showMore,...__sveltets_2_empty({"data-find-skip":true}),});
               Math.min(CHUNK, hiddenCount);   hiddenCount; 
             }
          }
         }
      }
    }
   }
}else{
   { svelteHTML.createElement("div", {   "class":`line leaf`,...__sveltets_2_empty({"data-jpath":path}),});
    if(label !== null){ { svelteHTML.createElement("span", {   "class":`k`,...__sveltets_2_empty({"data-find-skip":keySkip || undefined}),});label; } { svelteHTML.createElement("span", {  "class":`sep`,...__sveltets_2_empty({"data-find-skip":true}),});  }}
    if(bson !== null){
       { svelteHTML.createElement("span", { "class":`json-bson`,});bson; }
    } else if (value === null || value === undefined){
       { svelteHTML.createElement("span", { "class":`json-null`,});  }
    } else if (typeof value === 'string'){
       { svelteHTML.createElement("span", {  "class":`json-str`,}); strLong && !strOpen ? str.slice(0, STR_MAX) : str;strLong && !strOpen ? '…' : '';  }
      if(strLong){
         { svelteHTML.createElement("button", {      "class":`more inline`,"type":`button`,"onclick":toggleStr,...__sveltets_2_empty({"data-find-skip":true}),});
          strOpen ? 'less' : `${str.length - STR_MAX} more chars`;
         }
      }
    } else if (typeof value === 'number' || typeof value === 'bigint'){
       { svelteHTML.createElement("span", { "class":`json-num`,});value; }
    } else if (typeof value === 'boolean'){
       { svelteHTML.createElement("span", { "class":`json-bool`,});value; }
    }else{
       { svelteHTML.createElement("span", { "class":`json-str`,});String(value); }
    }
   }
}


};
return { props: {} as any as Props, exports: {}, bindings: __sveltets_$$bindings(''), slots: {}, events: {} }}
const JsonTree__SvelteComponent_ = __sveltets_2_fn_component($$render());
/*Ωignore_startΩ*/type JsonTree__SvelteComponent_ = ReturnType<typeof JsonTree__SvelteComponent_>;
/*Ωignore_endΩ*/export default JsonTree__SvelteComponent_;