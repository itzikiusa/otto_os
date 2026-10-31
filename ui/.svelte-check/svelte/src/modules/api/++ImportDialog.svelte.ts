///<reference types="svelte" />
;// One "Import" sheet for every way in: paste a curl command (opens it as a
// request tab), a Postman / OpenAPI / HAR file (becomes a collection), or a
// whole Postman account (every collection + environment via the Postman API).

import Icon from '../../lib/components/Icon.svelte';
import { onTabKey } from '../../lib/tabKeys';
import Modal from '../../lib/components/Modal.svelte';
import { apiClient } from '../../lib/stores/apiClient.svelte';
import { ws } from '../../lib/stores/workspace.svelte';
;

  type Mode = 'curl' | 'file' | 'postman';;;
  interface Props {
    initial?: Mode;
    onclose: () => void;
    /** Called after a curl import opened its tab (so the page can show it). */
    onimported?: () => void;
  };function $$render() {

  
  
  
  
  
  
  
  


  let { initial = 'curl', onclose, onimported }: Props = $props();

  // svelte-ignore state_referenced_locally
  let mode = $state<Mode>(initial);
  const canEdit = $derived(ws.myRole !== 'viewer');
  const MODES: { id: Mode; label: string }[] = [
    { id: 'curl', label: 'curl command' },
    { id: 'file', label: 'File' },
    { id: 'postman', label: 'Postman account' },
  ];

  let curl = $state('');
  let busy = $state(false);
  let pmKey = $state('');
  let pmRemember = $state(true);

  async function importCurl(): Promise<void> {
    busy = true;
    try {
      if (await apiClient.importCurl(curl)) {
        onimported?.();
        onclose();
      }
    } finally {
      busy = false;
    }
  }

  async function importFile(input: HTMLInputElement): Promise<void> {
    const file = input.files?.[0];
    input.value = '';
    if (!file) return;
    // Close right away — folders + requests are created one by one and the
    // result is reported with a toast.
    onclose();
    await apiClient.importFile(file);
  }

  async function syncPostman(): Promise<void> {
    busy = true;
    try {
      if (await apiClient.postmanSync(pmKey.trim(), pmRemember)) onclose();
    } finally {
      busy = false;
    }
  }

;
async () => {

 { const $$_ladoM0C = __sveltets_2_ensureComponent(Modal); const $$_ladoM0 = new $$_ladoM0C({ target: __sveltets_2_any(), props: {    children:() => { return __sveltets_2_any(0); },"title":`Import`,"width":520,onclose,footer:() => { async ()/*Ωignore_positionΩ*/ => {
     { svelteHTML.createElement("button", {   "class":`btn`,"onclick":onclose,});  }
    if(mode === 'curl'){
       { svelteHTML.createElement("button", {     "class":`btn primary`,"onclick":importCurl,"disabled":busy || !curl.trim(),});busy ? 'Importing…' : 'Open as request'; }
    } else if (mode === 'postman'){
       { svelteHTML.createElement("button", {     "class":`btn primary`,"onclick":syncPostman,"disabled":busy || !canEdit,});busy ? 'Importing…' : 'Import everything'; }
    }
  };return __sveltets_2_any(0)},}});/*Ωignore_startΩ*/const {footer} = $$_ladoM0.$$prop_def;/*Ωignore_endΩ*/
   { svelteHTML.createElement("div", { "class":`imp`,});
     { svelteHTML.createElement("div", {     "class":`segmented`,"role":`tablist`,"aria-label":`Import from`,});
         for(let m of __sveltets_2_ensureArray(MODES)){m.id;
         { svelteHTML.createElement("button", {          "role":`tab`,"aria-selected":mode === m.id,"tabindex":mode === m.id ? 0 : -1,"onclick":() => (mode = m.id),"onkeydown":onTabKey,});mode === m.id;m.label; }
      }
     }

    if(mode === 'curl'){
       { svelteHTML.createElement("p", { "class":`lead`,});   { svelteHTML.createElement("code", {});  }                   }
       { svelteHTML.createElement("textarea", {             "dir":`ltr`,"class":`input mono curl`,"rows":6,"bind:value":curl,"spellcheck":`false`,"aria-label":`curl command`,"placeholder":"curl https://api.example.com/v1/users \\\n  -H 'Authorization: Bearer {{api_token}}'",});/*Ωignore_startΩ*/() => curl = __sveltets_2_any(null);/*Ωignore_endΩ*/ }
       { svelteHTML.createElement("p", { "class":`hint`,});            }
    } else if (mode === 'file'){
       { svelteHTML.createElement("p", { "class":`lead`,});               { svelteHTML.createElement("strong", {});  }     { svelteHTML.createElement("strong", {});    }      { svelteHTML.createElement("strong", {});  }  }
       { svelteHTML.createElement("label", {  "class":`btn pick`,});!canEdit;
         { const $$_nocI3C = __sveltets_2_ensureComponent(Icon); new $$_nocI3C({ target: __sveltets_2_any(), props: {    "name":`file`,"size":13,}});} 
         { svelteHTML.createElement("input", {        "type":`file`,"accept":`.json,.har,.yaml,.yml`,"hidden":true,"disabled":!canEdit,"onchange":(e) => void importFile(e.currentTarget as HTMLInputElement),});}
       }
      if(!canEdit){ { svelteHTML.createElement("p", { "class":`hint`,});            }}
    }else{
       { svelteHTML.createElement("p", { "class":`lead`,});  { svelteHTML.createElement("strong", {});    }                         }
       { svelteHTML.createElement("div", { "class":`field`,});
         { svelteHTML.createElement("label", { "for":`pm-key`,});   }
         { svelteHTML.createElement("input", {              "id":`pm-key`,"class":`input`,"type":`password`,"bind:value":pmKey,"placeholder":`PMAK-…`,"autocomplete":`off`,"disabled":!canEdit,});/*Ωignore_startΩ*/() => pmKey = __sveltets_2_any(null);/*Ωignore_endΩ*/}
         { svelteHTML.createElement("span", { "class":`hint`,});         }
       }
       { svelteHTML.createElement("label", { "class":`checkbox-row`,}); { svelteHTML.createElement("input", {      "type":`checkbox`,"bind:checked":pmRemember,"disabled":!canEdit,});/*Ωignore_startΩ*/() => pmRemember = __sveltets_2_any(null);/*Ωignore_endΩ*/}        }
    }
   }
  
 Modal}


};
return { props: {} as any as Props, exports: {}, bindings: __sveltets_$$bindings(''), slots: {}, events: {} }}
const ImportDialog__SvelteComponent_ = __sveltets_2_fn_component($$render());
/*Ωignore_startΩ*/type ImportDialog__SvelteComponent_ = ReturnType<typeof ImportDialog__SvelteComponent_>;
/*Ωignore_endΩ*/export default ImportDialog__SvelteComponent_;