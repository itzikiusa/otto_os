///<reference types="svelte" />
;// Quick switcher (⌘O inside the vault): server-side fuzzy over
// title/aliases/path; Enter opens, Shift+Enter creates a note by that name.

import type { VaultSwitchHit } from '../../lib/api/types';
import { vault } from './vault.svelte';
import { loadErrorText } from '../../lib/loadError';
import { announce } from '../../lib/announce.svelte';
import Modal from '../../lib/components/Modal.svelte';
function $$render() {

  
  
  
  
  
  
  

  let query = $state('');
  let hits = $state<VaultSwitchHit[]>([]);
  let sel = $state(0);
  let input = $state<HTMLInputElement | undefined>();
  let seq = 0;
  let loading = $state(false);
  let lookupError = $state('');
  let resolvedQuery = $state<string | null>(null);
  const canCreate = $derived(!loading && !lookupError && resolvedQuery === query && hits.length === 0 && !!query.trim());

  $effect(() => {
    if (vault.switcherOpen) {
      query = '';
      hits = [];
      sel = 0;
      void refresh('');
      requestAnimationFrame(() => input?.focus());
    }
  });

  async function refresh(q: string): Promise<void> {
    const my = ++seq;
    const id = vault.current?.id, workspace = vault.wsId, generation = vault.lookupGeneration;
    const current = () => my === seq && query === q && vault.switcherOpen && vault.current?.id === id && vault.wsId === workspace && vault.lookupGeneration === generation;
    loading = true; lookupError = ''; resolvedQuery = null; hits = []; sel = 0;
    try {
      const got = await vault.switcherQuery(q);
      if (current()) { hits = got; sel = 0; resolvedQuery = q; }
    } catch (e) {
      if (current()) { lookupError = loadErrorText(e); announce(`Couldn’t search notes. ${lookupError}`, { assertive: true }); }
    } finally {
      if (current()) loading = false;
    }
  }

  function close(): void {
    vault.switcherOpen = false;
  }

  function pick(h: VaultSwitchHit | undefined): void {
    if (!h) return;
    close();
    void vault.open(h.path);
  }

  function createFromQuery(explicit = false): void {
    const name = query.trim();
    if (!name) return;
    if (!explicit && (loading || lookupError || resolvedQuery !== query || hits.length > 0)) return;
    close();
    void vault.createNote(name.endsWith('.md') ? name : `${name}.md`, `# ${name}\n\n`);
  }

  function onKey(e: KeyboardEvent): void {
    if (e.key === 'Escape') {
      e.preventDefault();
      close();
    } else if (e.key === 'ArrowDown') {
      e.preventDefault();
      sel = Math.max(0, Math.min(sel + 1, hits.length - 1));
    } else if (e.key === 'ArrowUp') {
      e.preventDefault();
      sel = Math.max(sel - 1, 0);
    } else if (e.key === 'Enter') {
      e.preventDefault();
      if (e.shiftKey) createFromQuery(true);
      else if (loading || lookupError || resolvedQuery !== query) return;
      else if (hits.length === 0) createFromQuery();
      else pick(hits[sel]);
    }
  }
;
async () => {

if(vault.switcherOpen){
   { const $$_ladoM0C = __sveltets_2_ensureComponent(Modal); new $$_ladoM0C({ target: __sveltets_2_any(), props: {     children:() => { return __sveltets_2_any(0); },"title":`Quick switcher`,"width":620,"onclose":close,}});
     { svelteHTML.createElement("div", { "class":`vs-body`,});
       { const $$_input2 = svelteHTML.createElement("input", {                         "dir":`auto`,"bind:value":query,"class":`vs-input`,"placeholder":`Open note… (Shift+Enter creates)`,"aria-label":`Open note`,"role":`combobox`,"aria-autocomplete":`list`,"aria-controls":`vs-list`,"aria-expanded":hits.length > 0 || canCreate,"aria-activedescendant":hits.length > 0 ? `vs-opt-${sel}` : canCreate ? 'vs-opt-create' : undefined,"oninput":() => void refresh(query),"onkeydown":onKey,});input = $$_input2;/*Ωignore_startΩ*/() => query = __sveltets_2_any(null);/*Ωignore_endΩ*/}
      if(loading){
         { svelteHTML.createElement("div", {});  }
      } else if (lookupError){
         { svelteHTML.createElement("div", { "class":`lookup-error`,});   lookupError; }
         { svelteHTML.createElement("button", {   "class":`btn small`,"onclick":() => void refresh(query),});  }
      }
      
       { svelteHTML.createElement("div", {       "class":`hits`,"id":`vs-list`,"role":`listbox`,"aria-label":`Matching notes`,});
            for(let h of __sveltets_2_ensureArray((hits.slice(0, 30)))){let i = 1;h.path + (h.alias ?? '');
           { svelteHTML.createElement("button", {            "class":`hit`,"id":`vs-opt-${i}`,"role":`option`,"aria-selected":i === sel,"tabindex":-1,"onclick":() => pick(h),});i === sel;
             { svelteHTML.createElement("span", { "class":`t`,});h.alias ?? h.title; }
            if(h.alias){ { svelteHTML.createElement("span", { "class":`via`,}); h.title; }}
             { svelteHTML.createElement("span", {   "class":`p`,"title":h.path,});h.path; }
           }
        }
        if(canCreate){
           { svelteHTML.createElement("button", {           "class":`hit create-btn sel`,"id":`vs-opt-create`,"role":`option`,"aria-selected":`true`,"tabindex":-1,"onclick":() => createFromQuery(),});
             { svelteHTML.createElement("span", { "class":`t`,}); query.trim();  }
             { svelteHTML.createElement("span", { "class":`p`,});    }
           }
        }
       }
     }
   Modal}
}


};
return { props: {} as Record<string, never>, exports: {}, bindings: __sveltets_$$bindings(''), slots: {}, events: {} }}
const Switcher__SvelteComponent_ = __sveltets_2_fn_component($$render());
/*Ωignore_startΩ*/type Switcher__SvelteComponent_ = ReturnType<typeof Switcher__SvelteComponent_>;
/*Ωignore_endΩ*/export default Switcher__SvelteComponent_;