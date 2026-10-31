///<reference types="svelte" />
;
import { vault } from './vault.svelte';
import { PROPERTY_FIELDS, patchProperties, readProperties } from './properties';
function $$render() {

  
  
  let editing = $state(false);
  let base = $state('');
  let values = $state<Record<string, string>>({});
  let error = $state('');
  const preview = $derived.by(() => {
    if (!editing) return '';
    try { return patchProperties(base, values); } catch { return ''; }
  });
  function begin() {
    error = '';
    try {
      base = vault.draft;
      values = readProperties(base);
      editing = true;
    } catch (e) { error = String(e); }
  }
  async function apply() {
    if (vault.draft !== base) { error = 'This note changed. Cancel and reopen properties to use the latest version.'; return; }
    if (!preview) return;
    vault.onDraftChange(preview);
    if (await vault.saveNow()) editing = false;
    else error = 'Properties remain in your draft. Resolve the save error or conflict before leaving.';
  }
;
async () => {
if(error){ { svelteHTML.createElement("p", {   "class":`error`,"role":`alert`,});error; }}
if(editing){
   { svelteHTML.createElement("form", { "onsubmit":(e) => {e.preventDefault(); void apply();},});
       for(let key of __sveltets_2_ensureArray(PROPERTY_FIELDS)){key;
       { svelteHTML.createElement("label", {});key; { svelteHTML.createElement("input", {        "dir":`auto`,"aria-label":`Property ${key}`,"bind:value":values[key],"placeholder":key === 'tags' || key === 'aliases' ? 'Comma-separated values' : '',});/*Ωignore_startΩ*/() => values[key] = __sveltets_2_any(null);/*Ωignore_endΩ*/} }
    }
     { svelteHTML.createElement("details", {}); { svelteHTML.createElement("summary", {});  } { svelteHTML.createElement("pre", {});preview; } }
     { svelteHTML.createElement("div", { "class":`actions`,}); { svelteHTML.createElement("button", {   "type":`submit`,"disabled":vault.saving || vault.conflict,});  } { svelteHTML.createElement("button", {   "type":`button`,"onclick":() => {editing = false; error = '';},});  } }
   }
}else{
   { svelteHTML.createElement("button", {     "class":`edit`,"disabled":!vault.note || vault.note.meta.parse_error,"onclick":begin,});  }
}

};
return { props: {} as Record<string, never>, exports: {}, bindings: __sveltets_$$bindings(''), slots: {}, events: {} }}
const PropertiesEditor__SvelteComponent_ = __sveltets_2_fn_component($$render());
/*Ωignore_startΩ*/type PropertiesEditor__SvelteComponent_ = ReturnType<typeof PropertiesEditor__SvelteComponent_>;
/*Ωignore_endΩ*/export default PropertiesEditor__SvelteComponent_;