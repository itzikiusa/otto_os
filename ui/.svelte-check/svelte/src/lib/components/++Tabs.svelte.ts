///<reference types="svelte" />
;
  import type { IconName } from './Icon.svelte';

  export interface TabItem<I extends string = string> {
    id: I;
    label: string;
    icon?: IconName;
    /** Shown after the label in --text-dim ("Pending 3"). */
    count?: number | string;
    disabled?: boolean;
    /** Tooltip — e.g. why a tab is disabled. */
    title?: string;
    /** data-testid on this tab's button. */
    testid?: string;
  }
;;// The one in-pane tab strip (components.md §3): an underline under the
// selected tab, `role="tablist"` + `role="tab"` / `aria-selected`, roving
// tabindex with ←/→ (visual direction — RTL flips), Home/End, disabled tabs
// skipped (lib/tabKeys.ts). Page-level tabs still go in PageHeader's `tabs`
// snippet; 2–5 options that pick a VALUE stay a `.segmented`.
//
//   <Tabs label="Queue details" tabs={[{ id: 'messages', label: 'Messages' }, …]}
//         value={tab} onchange={(id) => (tab = id)} idBase="sqs" />
//   <div role="tabpanel" id="sqs-panel-{tab}" aria-labelledby="sqs-tab-{tab}">…</div>
//
// `onchange` is the single place that switches the view, for pointer and
// keyboard alike (arrow keys click the next tab). Pass `activate: 'click'`
// when the switch is async or can be refused (an unsaved-changes guard), so
// focus never lands on a tab that didn't take.


import type { Snippet } from 'svelte';
import Icon from './Icon.svelte';
import { tabKeys, type TabKeyOptions } from '../tabKeys';
function $$render<T extends string>() {

  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  

  interface Props {
    tabs: readonly TabItem<T>[];
    value: T;
    onchange: (id: T) => void;
    /** aria-label of the tablist ("Queue details"). */
    label: string;
    /** When set, tab `i` gets id `{idBase}-tab-{i}` and aria-controls
     *  `{idBase}-panel-{i}` — give the panel that id + aria-labelledby. */
    idBase?: string;
    size?: 'm' | 's';
    activate?: TabKeyOptions['activate'];
    /** Extra controls at the end of the strip (outside the tablist). */
    trailing?: Snippet;
    testid?: string;
  }
  let { tabs, value, onchange, label, idBase, size = 'm', activate = 'focus', trailing, testid }: Props = $props();
  const onkeydown = $derived(tabKeys({ activate }));

  // ── Overflow affordance (S19-303) ─────────────────────────────────────────
  // The strip scrolls horizontally with its scrollbar hidden, so a hidden tab
  // used to give no hint it existed: fade the edge(s) that hide tabs, map a
  // plain mouse wheel to horizontal scroll, keep the selected tab in view, and
  // move the `trailing` controls to their own row when tabs + trailing don't
  // fit side by side (measured from the tabs' own widths, so it can't flap).
  let rootEl = $state<HTMLDivElement>();
  let listEl = $state<HTMLDivElement>();
  let trailEl = $state<HTMLDivElement>();
  let fadeStart = $state(false);
  let fadeEnd = $state(false);
  let wrapTrailing = $state(false);

  function measure(): void {
    const list = listEl;
    if (!list) return;
    const max = list.scrollWidth - list.clientWidth;
    const rtl = getComputedStyle(list).direction === 'rtl';
    const pos = rtl ? -list.scrollLeft : list.scrollLeft; // 0 = at the start edge
    fadeStart = max > 1 && pos > 1;
    fadeEnd = max > 1 && pos < max - 1;
    const root = rootEl;
    const trail = trailEl;
    if (root && trail) {
      let tabsW = 0;
      for (const el of Array.from(list.children)) tabsW += (el as HTMLElement).offsetWidth + 2;
      const cs = getComputedStyle(root);
      const inner = root.clientWidth - parseFloat(cs.paddingInlineStart || '0') - parseFloat(cs.paddingInlineEnd || '0');
      wrapTrailing = tabsW + trail.offsetWidth + 8 > inner;
    } else {
      wrapTrailing = false;
    }
  }

  $effect(() => {
    void tabs; // re-observe when the tab set changes
    const root = rootEl;
    const list = listEl;
    if (!root || !list || typeof ResizeObserver === 'undefined') return;
    const ro = new ResizeObserver(() => measure());
    ro.observe(root);
    ro.observe(list);
    for (const el of Array.from(list.children)) ro.observe(el);
    return () => ro.disconnect();
  });

  // Keep the selected tab visible when `value` changes in code (and after the
  // tab list itself changes); `nearest` never scrolls the page vertically.
  $effect(() => {
    void value;
    void tabs.length;
    const list = listEl;
    if (!list) return;
    queueMicrotask(() => {
      const on = list.querySelector<HTMLElement>('[aria-selected="true"]');
      on?.scrollIntoView?.({ block: 'nearest', inline: 'nearest' });
      measure();
    });
  });

  /** A vertical mouse wheel scrolls an overflowing strip sideways (a
   *  non-passive listener: it has to preventDefault the page scroll). */
  $effect(() => {
    const list = listEl;
    if (!list) return;
    list.addEventListener('wheel', onwheel, { passive: false });
    return () => list.removeEventListener('wheel', onwheel);
  });
  function onwheel(e: WheelEvent): void {
    const list = listEl;
    if (!list || list.scrollWidth <= list.clientWidth) return;
    if (Math.abs(e.deltaY) <= Math.abs(e.deltaX)) return;
    list.scrollLeft += getComputedStyle(list).direction === 'rtl' ? -e.deltaY : e.deltaY;
    e.preventDefault();
  }
;
async () => {



 { const $$_div0 = svelteHTML.createElement("div", {    "class":`otabs`,});size === 's';wrapTrailing;rootEl = $$_div0;
   { const $$_div1 = svelteHTML.createElement("div", {              "class":`otabs-list`,"role":`tablist`,"aria-label":label,...__sveltets_2_empty({"data-testid":testid}),"onscroll":measure,});fadeStart;fadeEnd;listEl = $$_div1;
       for(let t of __sveltets_2_ensureArray(tabs)){t.id;
       { svelteHTML.createElement("button", {                         "type":`button`,"role":`tab`,"class":`otab`,"id":idBase ? `${idBase}-tab-${t.id}` : undefined,"aria-controls":idBase ? `${idBase}-panel-${t.id}` : undefined,"aria-selected":t.id === value,"tabindex":t.id === value ? 0 : -1,"disabled":t.disabled,"title":t.title || undefined,...__sveltets_2_empty({"data-testid":t.testid}),"onclick":() => {
          if (t.id !== value) onchange(t.id);
        },onkeydown,});t.id === value;
        if(t.icon){ { const $$_nocI3C = __sveltets_2_ensureComponent(Icon); new $$_nocI3C({ target: __sveltets_2_any(), props: {    "name":t.icon,"size":size === 's' ? 12 : 13,}});}}
         { svelteHTML.createElement("span", { "class":`otab-label`,});t.label; }
        if(t.count !== undefined && t.count !== ''){ { svelteHTML.createElement("span", { "class":`otab-count`,});t.count; }}
       }
    }
   }
  if(trailing){ { const $$_div1 = svelteHTML.createElement("div", {  "class":`otabs-trailing`,});trailEl = $$_div1;;__sveltets_2_ensureSnippet(trailing()); }}
 }


};
return { props: {} as any as Props, exports: {}, bindings: __sveltets_$$bindings(''), slots: {}, events: {} }}
class __sveltets_Render<T extends string> {
    props(): ReturnType<typeof $$render<T>>['props'] { return null as any; }
    events(): ReturnType<typeof $$render<T>>['events'] { return null as any; }
    slots(): ReturnType<typeof $$render<T>>['slots'] { return null as any; }
    bindings() { return __sveltets_$$bindings(''); }
    exports() { return {}; }
}

interface $$IsomorphicComponent {
    new <T extends string>(options: import('svelte').ComponentConstructorOptions<ReturnType<__sveltets_Render<T>['props']>>): import('svelte').SvelteComponent<ReturnType<__sveltets_Render<T>['props']>, ReturnType<__sveltets_Render<T>['events']>, ReturnType<__sveltets_Render<T>['slots']>> & { $$bindings?: ReturnType<__sveltets_Render<T>['bindings']> } & ReturnType<__sveltets_Render<T>['exports']>;
    <T extends string>(internal: unknown, props: ReturnType<__sveltets_Render<T>['props']> & {}): ReturnType<__sveltets_Render<T>['exports']>;
    z_$$bindings?: ReturnType<__sveltets_Render<any>['bindings']>;
}
const Tabs__SvelteComponent_: $$IsomorphicComponent = null as any;
/*Ωignore_startΩ*/type Tabs__SvelteComponent_<T extends string> = InstanceType<typeof Tabs__SvelteComponent_<T>>;
/*Ωignore_endΩ*/export default Tabs__SvelteComponent_;