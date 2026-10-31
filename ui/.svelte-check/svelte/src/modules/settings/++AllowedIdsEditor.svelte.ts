///<reference types="svelte" />
;// Settings → Channels: the allowed-users list as a LIST (one row per id,
// with the Slack member's name when the directory knows it) instead of a
// comma-separated text box. The comma string stays the value — it is what
// the API stores — so this is a view + editor over `value`.

import { tick } from 'svelte';
import Icon from '../../lib/components/Icon.svelte';
import { ctxMenu, type MenuItem } from '../../lib/contextmenu.svelte';
import type { SlackDirUser } from '../../lib/api/types';
import { allowedIds, withAllowedMany, withoutAllowed } from './channelAllow';
;

  interface Props {
    /** Comma-separated ids (bindable). */
    value: string;
    /** Slack members for names + "Add person…"; empty → ids only. */
    people?: SlackDirUser[];
    placeholder?: string;
    /** Platform word for the empty state ("Slack", "Telegram"). */
    label: string;
    /** Id of the visible heading that names this group. */
    labelledby?: string;
  };function $$render() {

  
  
  
  
  
  
  
  
  

  let { value = $bindable(), people = [], placeholder = 'User ID', label, labelledby }: Props = $props()/*Ωignore_startΩ*/;value;/*Ωignore_endΩ*/;

  const ids = $derived(allowedIds(value));
  const byId = $derived(new Map(people.map((p) => [p.id, p])));
  let typed = $state('');
  let listEl = $state<HTMLElement>();
  let addInput = $state<HTMLInputElement>();

  /** Remove row `i`, then keep focus in the list: the same index's remove
   *  button (the next row slid up), else the previous one, else the add box —
   *  never `<body>` (the keyed row holding the focused button is destroyed). */
  async function remove(id: string, i: number): Promise<void> {
    value = withoutAllowed(value, id);
    await tick();
    const btns = listEl?.querySelectorAll<HTMLButtonElement>('button.rm');
    (btns?.[i] ?? btns?.[i - 1] ?? addInput)?.focus();
  }

  function addTyped(): void {
    if (!typed.trim()) return;
    value = withAllowedMany(value, typed);
    typed = '';
  }

  function pickPerson(e: MouseEvent | KeyboardEvent): void {
    const taken = new Set(ids);
    const items: MenuItem[] = people
      .filter((p) => !taken.has(p.id))
      .sort((a, b) => a.name.localeCompare(b.name))
      .map((p) => ({
        label: p.name,
        hint: p.is_bot ? `@${p.handle} · bot` : `@${p.handle}`,
        title: p.id,
        action: () => (value = withAllowedMany(value, p.id)),
      }));
    if (items.length === 0) items.push({ label: 'Everyone is already allowed', disabled: true });
    ctxMenu.show(e, items, { filter: true, filterPlaceholder: 'Search people…', maxVisible: 12 });
  }
;
async () => {

 { svelteHTML.createElement("div", {       "class":`allow-list`,"role":`group`,"aria-labelledby":labelledby,...__sveltets_2_empty({"data-testid":`allowed-ids`}),});
  if(ids.length > 0){
     { const $$_ul1 = svelteHTML.createElement("ul", {  "aria-label":`Allowed user list`,});listEl = $$_ul1;
          for(let id of __sveltets_2_ensureArray(ids)){let i = 1;id;
        const p = byId.get(id);
         { svelteHTML.createElement("li", {});
           { const $$_nocI3C = __sveltets_2_ensureComponent(Icon); new $$_nocI3C({ target: __sveltets_2_any(), props: {    "name":p?.is_bot ? 'sparkle' : 'user',"size":13,}});}
          if(p){
             { svelteHTML.createElement("span", { "class":`who ellipsis`,});p.name; }
             { svelteHTML.createElement("span", { "class":`handle ellipsis`,}); p.handle; }
          }else{
             { svelteHTML.createElement("span", {   "class":`who unknown ellipsis`,"title":`No ${label} name found for this id`,});  }
          }
           { svelteHTML.createElement("span", {   "class":`id mono`,"dir":`ltr`,});id; }
           { svelteHTML.createElement("button", {           "type":`button`,"class":`rm`,"aria-label":`Remove ${p?.name ?? id}`,"title":`Remove from the allowed users`,"onclick":() => void remove(id, i),});
             { const $$_nocI4C = __sveltets_2_ensureComponent(Icon); new $$_nocI4C({ target: __sveltets_2_any(), props: {    "name":`x`,"size":12,}});}
           }
         }
      }
     }
  }else{
     { svelteHTML.createElement("p", { "class":`empty`,});    }
  }
   { svelteHTML.createElement("div", { "class":`add-row`,});
    if(people.length > 0){
       { svelteHTML.createElement("button", {       "type":`button`,"class":`btn small`,"onclick":pickPerson,"aria-haspopup":`menu`,});
         { const $$_nocI3C = __sveltets_2_ensureComponent(Icon); new $$_nocI3C({ target: __sveltets_2_any(), props: {    "name":`plus`,"size":12,}});}  
       }
    }
     { const $$_input2 = svelteHTML.createElement("input", {                   "dir":`ltr`,"class":`input mono add-id`,"aria-label":`Add user id`,"bind:value":typed,placeholder,"spellcheck":`false`,"autocomplete":`off`,"onblur":() => {
        // Leaving the box keeps a finished id (so Save doesn't drop it), not
        // half-typed text.
        if (typed.split(/[\s,]+/).every((t) => t === '' || /^[A-Za-z0-9_-]{4,}$/.test(t))) addTyped();
      },"onkeydown":(e) => {
        if (e.key === 'Enter') {
          e.preventDefault();
          addTyped();
        }
      },});addInput = $$_input2;/*Ωignore_startΩ*/() => typed = __sveltets_2_any(null);/*Ωignore_endΩ*/}
     { svelteHTML.createElement("button", {       "type":`button`,"class":`btn small`,"disabled":!typed.trim(),"onclick":addTyped,});  }
   }
 }


};
return { props: {} as any as Props, exports: {}, bindings: __sveltets_$$bindings('value'), slots: {}, events: {} }}
const AllowedIdsEditor__SvelteComponent_ = __sveltets_2_fn_component($$render());
/*Ωignore_startΩ*/type AllowedIdsEditor__SvelteComponent_ = ReturnType<typeof AllowedIdsEditor__SvelteComponent_>;
/*Ωignore_endΩ*/export default AllowedIdsEditor__SvelteComponent_;