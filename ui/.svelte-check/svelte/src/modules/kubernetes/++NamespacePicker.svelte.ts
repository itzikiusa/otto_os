///<reference types="svelte" />
;// Namespace combobox: type to filter, "All namespaces" pinned first, arrow
// keys + Enter/Esc. The popup is `position: fixed`, CLAMPED into the viewport
// and height-capped (scrolls) so a cluster with hundreds of namespaces stays
// reachable (AGENTS.md "Floating UI must survive long content"). Phones get
// a native <select> instead.

import { tick } from 'svelte';
import Icon from '../../lib/components/Icon.svelte';
import { viewport } from '../../lib/stores/viewport.svelte';
import type { K8sNamespace } from '../../lib/api/types';
;

  interface Props {
    value: string;
    namespaces: K8sNamespace[];
    /** Non-empty when the namespace list failed to load — typing still works. */
    error?: string;
    disabled?: boolean;
    allowAll?: boolean;
    onchange: (ns: string) => void;
  };function $$render() {

  
  
  
  
  
  
  
  
  

  let { value, namespaces, error = '', disabled = false, allowAll = true, onchange }: Props = $props();

  const ALL = '';
  let open = $state(false);
  let query = $state('');
  let active = $state(0);
  let inputEl = $state<HTMLInputElement | null>(null);
  let popEl = $state<HTMLDivElement | null>(null);
  let pos = $state({ top: 0, left: 0, width: 240, maxH: 320 });

  const options = $derived.by(() => {
    const q = query.trim().toLowerCase();
    const names = namespaces.map((n) => n.name).filter((n) => !q || n.toLowerCase().includes(q));
    const list: { value: string; label: string }[] = [];
    if (allowAll && (!q || 'all namespaces'.includes(q))) list.push({ value: ALL, label: 'All namespaces' });
    for (const n of names) list.push({ value: n, label: n });
    // Let the user type a namespace the list doesn't know (RBAC-limited
    // `get namespaces`, or the list failed) and still select it.
    // Namespaces are lowercase DNS labels — normalize the free-typed one so a
    // phone's auto-capitalized "Mscasino" doesn't turn into a 403.
    if (q && !names.some((n) => n.toLowerCase() === q)) list.push({ value: q, label: `Use “${q}”` });
    return list;
  });

  const shown = $derived(value === ALL ? 'All namespaces' : value);

  function place(): void {
    if (!inputEl) return;
    const r = inputEl.getBoundingClientRect();
    const pad = 8;
    const vw = window.innerWidth;
    const vh = window.innerHeight;
    const width = Math.max(220, Math.min(360, r.width));
    const left = Math.max(pad, Math.min(r.left, vw - width - pad));
    const below = vh - r.bottom - pad;
    // Prefer below; if there's more room above, open upward but keep it capped
    // and INSIDE the viewport (never a negative top).
    const above = r.top - pad;
    const wantH = 320;
    let top: number;
    let maxH: number;
    if (below >= Math.min(wantH, 160) || below >= above) {
      top = r.bottom + 4;
      maxH = Math.max(120, Math.min(wantH, below - 4));
    } else {
      maxH = Math.max(120, Math.min(wantH, above - 4));
      top = Math.max(pad, r.top - 4 - maxH);
    }
    // The 120px floor can still overrun a short window — keep the whole popup
    // on screen (and never above the top edge).
    maxH = Math.min(maxH, vh - 2 * pad);
    top = Math.max(pad, Math.min(top, vh - pad - maxH));
    pos = { top, left, width, maxH };
  }

  async function show(): Promise<void> {
    if (disabled) return;
    query = '';
    active = 0;
    open = true;
    await tick();
    place();
  }

  function choose(v: string): void {
    open = false;
    query = '';
    if (v !== value) onchange(v);
  }

  function onKey(e: KeyboardEvent): void {
    if (!open && (e.key === 'ArrowDown' || e.key === 'Enter')) {
      e.preventDefault();
      void show();
      return;
    }
    if (!open) return;
    if (e.key === 'ArrowDown') {
      e.preventDefault();
      active = Math.min(options.length - 1, active + 1);
      scrollActive();
    } else if (e.key === 'ArrowUp') {
      e.preventDefault();
      active = Math.max(0, active - 1);
      scrollActive();
    } else if (e.key === 'Enter') {
      e.preventDefault();
      const o = options[active];
      if (o) choose(o.value);
    } else if (e.key === 'Escape') {
      e.preventDefault();
      e.stopPropagation();
      open = false;
      query = '';
    }
  }

  function scrollActive(): void {
    popEl?.querySelector<HTMLElement>(`[data-i="${active}"]`)?.scrollIntoView({ block: 'nearest' });
  }

  function onInput(): void {
    active = 0;
    if (!open) void show();
  }

  // Close on outside pointerdown + keep placed on resize/scroll.
  $effect(() => {
    if (!open) return;
    const down = (e: PointerEvent): void => {
      const t = e.target as Node;
      if (popEl?.contains(t) || inputEl?.contains(t)) return;
      open = false;
      query = '';
    };
    const re = (): void => place();
    window.addEventListener('pointerdown', down, true);
    window.addEventListener('resize', re);
    window.addEventListener('scroll', re, true);
    return () => {
      window.removeEventListener('pointerdown', down, true);
      window.removeEventListener('resize', re);
      window.removeEventListener('scroll', re, true);
    };
  });

  /** Let the workspace focus the field (`n` shortcut). */
   function focus(): void {
    inputEl?.focus();
    void show();
  }
;
async () => {

if(viewport.isPhone){
   { svelteHTML.createElement("select", {            "class":`input ns-select`,"aria-label":`Namespace`,disabled,"value":value,"onchange":(e) => onchange((e.currentTarget as HTMLSelectElement).value),...__sveltets_2_empty({"data-testid":`k8s-ns-picker`}),});
    if(allowAll){ { svelteHTML.createElement("option", {"value":"",});  }}
       for(let n of __sveltets_2_ensureArray(namespaces)){n.name; { svelteHTML.createElement("option", { "value":n.name,});n.name; }}
    if(value && !namespaces.some((n) => n.name === value)){ { svelteHTML.createElement("option", { "value":value,});value; }}
   }
}else{
   { svelteHTML.createElement("div", {    "class":`ns`,...__sveltets_2_empty({"data-testid":`k8s-ns-picker`}),});!!error;
    if(error){
       { svelteHTML.createElement("span", {   "class":`ns-warn`,"title":`Namespaces couldn’t be listed — type one to use it`,}); { const $$_nocI2C = __sveltets_2_ensureComponent(Icon); new $$_nocI2C({ target: __sveltets_2_any(), props: {    "name":`warning`,"size":13,}});} }
    }else{
       { const $$_nocI1C = __sveltets_2_ensureComponent(Icon); new $$_nocI1C({ target: __sveltets_2_any(), props: {    "name":`layers`,"size":13,}});}
    }
     { const $$_input1 = svelteHTML.createElement("input", {                                        "dir":`ltr`,"class":`ns-input`,"role":`combobox`,"aria-label":`Namespace`,"aria-expanded":open,"aria-controls":`k8s-ns-listbox`,"aria-autocomplete":`list`,"aria-activedescendant":open ? `k8s-ns-opt-${active}` : undefined,"placeholder":shown,"autocapitalize":`off`,"autocorrect":`off`,"spellcheck":false,"title":disabled ? 'This kind is cluster-scoped — no namespace applies' : error ? `Namespaces couldn’t be listed: ${error}` : shown,"value":open ? query : shown,disabled,"onblur":() => { open = false; query = ''; },"onfocus":() => void show(),"onclick":() => void show(),"oninput":(e) => { query = (e.currentTarget as HTMLInputElement).value; onInput(); },"onkeydown":onKey,});inputEl = $$_input1;}
     { const $$_nocI1C = __sveltets_2_ensureComponent(Icon); new $$_nocI1C({ target: __sveltets_2_any(), props: {    "name":`chevronDown`,"size":12,}});}
   }
  if(open){
     { const $$_div0 = svelteHTML.createElement("div", {            "id":`k8s-ns-listbox`,"class":`ns-pop`,"role":`listbox`,"aria-label":`Namespaces`,"style":`top:${pos.top}px;left:${pos.left}px;width:${pos.width}px;max-height:${pos.maxH}px`,});popEl = $$_div0;
      if(error){ { svelteHTML.createElement("div", { "class":`ns-err`,});               }}
          for(let o of __sveltets_2_ensureArray(options)){let i = 1;o.value + ':' + o.label;
         { svelteHTML.createElement("div", {                     "id":`k8s-ns-opt-${i}`,...__sveltets_2_empty({"data-i":i}),"class":`ns-opt`,"role":`option`,"aria-selected":o.value === value,"tabindex":-1,"onpointerenter":() => (active = i),"onpointerdown":(e) => { e.preventDefault(); choose(o.value); },"onkeydown":(e) => { if (e.key === 'Enter') choose(o.value); },});i === active;o.value === value;
           { svelteHTML.createElement("span", {    "class":`ns-opt-label`,"title":o.label,});o.value !== '';o.label; }
          if(o.value === value){ { const $$_nocI2C = __sveltets_2_ensureComponent(Icon); new $$_nocI2C({ target: __sveltets_2_any(), props: {    "name":`check`,"size":12,}});}}
         }
      }
      if(!options.length){ { svelteHTML.createElement("div", { "class":`ns-err`,});  }}
     }
  }
}


};
return { props: {} as any as Props, exports: {} as any as { focus: typeof focus }, bindings: __sveltets_$$bindings(''), slots: {}, events: {} }}
const NamespacePicker__SvelteComponent_ = __sveltets_2_fn_component($$render());
/*Ωignore_startΩ*/type NamespacePicker__SvelteComponent_ = ReturnType<typeof NamespacePicker__SvelteComponent_>;
/*Ωignore_endΩ*/export default NamespacePicker__SvelteComponent_;