///<reference types="svelte" />
;// Import Story dialog — choose a Jira/Linear/Confluence account, pick source kind,
// search for and pick a source, optionally set cwd + watch. Calls product.importStory()
// then selects the new story and closes.

import { untrack } from 'svelte';
import Modal from '../../lib/components/Modal.svelte';
import LoadState from '../../lib/components/LoadState.svelte';
import Icon from '../../lib/components/Icon.svelte';
import PathField from '../../lib/components/PathField.svelte';
import { issueAccounts } from '../../lib/stores/issueAccounts.svelte';
import { product } from '../../lib/stores/product.svelte';
import { toasts } from '../../lib/toast.svelte';
import { router } from '../../lib/router.svelte';
import { loadErrorText } from '../../lib/loadError';
import type { IssueAccount } from '../../lib/api/types';
import SourceSearch from './SourceSearch.svelte';
;

  interface Props {
    onclose: () => void;
    onimported?: () => void;
  };function $$render() {

  
  
  
  
  
  
  
  
  
  
  
  
  
  
  

  let { onclose, onimported }: Props = $props();

  // ── form state ────────────────────────────────────────────────────────────
  let accounts: IssueAccount[] = $state([]);
  let accountsLoading = $state(true);
  let accountsError = $state('');

  let accountId = $state('');
  let sourceKind: 'jira' | 'confluence' = $state('jira');
  // A Linear account has issues only ('jira' is the tracker-issue kind for
  // either provider; the account decides where it is fetched from).
  const accountProvider = $derived(accounts.find((a) => a.id === accountId)?.provider ?? 'jira');
  const isLinear = $derived(accountProvider === 'linear');
  $effect(() => {
    if (isLinear && sourceKind === 'confluence') sourceKind = 'jira';
  });

  // Picked via SourceSearch
  let selectedKey = $state('');
  let selectedLabel = $state('');

  // Manual fallback
  let showManual = $state(false);
  let manualKey = $state('');

  let cwd = $state('');
  let watchEnabled = $state(false);

  let submitting = $state(false);
  let formError = $state('');


  // ── load accounts on mount ────────────────────────────────────────────────
  // Untracked: the loader reads the account cache gate synchronously, and a
  // tracked read would re-run it once a cold cache fills.
  $effect(() => {
    untrack(() => void loadAccounts());
  });

  async function loadAccounts(): Promise<void> {
    accountsLoading = true;
    accountsError = '';
    try {
      accounts = await issueAccounts.load();
      if (accounts.length > 0) accountId = accounts[0].id;
    } catch (e) {
      accountsError = loadErrorText(e);
    } finally {
      accountsLoading = false;
    }
  }

  // ── reset the picked selection when the account or source kind changes ─────
  // Explicit handler (NOT an $effect): an $effect that writes selectedKey could
  // re-run and spuriously clear a valid pick. This only fires on a real change.
  function resetSelection(): void {
    selectedKey = '';
    selectedLabel = '';
    manualKey = '';
    showManual = false;
  }

  // ── helpers ───────────────────────────────────────────────────────────────

  // Normalise a raw user-entered key or URL:
  //  · Jira: extract PROJ-123 from a Jira browse URL (e.g. .../browse/PROJ-123)
  //          or from arbitrary pasted text that contains an issue key pattern.
  //  · Confluence: strip to numeric pageId from ?pageId=NNN or /pages/NNN/.
  function normaliseSourceKey(raw: string): string {
    const s = raw.trim();
    if (sourceKind === 'jira') {
      // https://xxx.atlassian.net/browse/PROJ-123 or text containing PROJ-123
      const m = s.match(/(?:\/browse\/|^|\s)([A-Z][A-Z0-9]{1,9}-\d+)(?:[/?#\s]|$)/);
      if (m) return m[1];
      return s;
    }
    // Confluence: Match ?pageId=NNN or /pages/NNN/
    const m = s.match(/(?:pageId=|\/pages\/)(\d+)/);
    if (m) return m[1];
    return s;
  }

  // The effective key submitted: manual field wins if filled, else selectedKey.
  const effectiveKey = $derived(
    showManual && manualKey.trim() ? normaliseSourceKey(manualKey) : selectedKey,
  );

  async function submit(): Promise<void> {
    formError = '';
    const key = effectiveKey;
    if (!accountId) { formError = 'Pick an account.'; return; }
    if (!key) { formError = 'Pick an issue or page, or enter its key manually.'; return; }

    submitting = true;
    try {
      const story = await product.importStory({
        source_kind: sourceKind,
        account_id: accountId,
        source_key: key,
        cwd: cwd.trim() || null,
        watch_enabled: watchEnabled,
      });
      // Select the newly imported story and switch to Overview tab.
      await product.select(story.id);
      product.tab = 'overview';
      onimported?.();
      onclose();
    } catch (e) {
      formError = loadErrorText(e);
    } finally {
      submitting = false;
    }
  }
;
async () => {

 { const $$_ladoM0C = __sveltets_2_ensureComponent(Modal); const $$_ladoM0 = new $$_ladoM0C({ target: __sveltets_2_any(), props: {    "title":`Import story`,"width":480,onclose,children:() => { async ()/*Ωignore_positionΩ*/ => {
    if(accountsLoading || accountsError){
       { const $$_etatSdaoL0C = __sveltets_2_ensureComponent(LoadState); new $$_etatSdaoL0C({ target: __sveltets_2_any(), props: {            "what":`your Jira / Linear / Confluence accounts`,"variant":`compact`,"loading":accountsLoading,"error":accountsError,"empty":true,"onretry":() => void loadAccounts(),}});}
    } else if (accounts.length === 0){
       { svelteHTML.createElement("div", { "class":`no-accounts`,});
         { const $$_nocI1C = __sveltets_2_ensureComponent(Icon); new $$_nocI1C({ target: __sveltets_2_any(), props: {    "name":`ticket`,"size":16,}});}
         { svelteHTML.createElement("span", {});                }
       }
       { svelteHTML.createElement("button", {   "class":`btn small`,"onclick":() => { onclose(); router.go('settings/jira'); },});
         { const $$_nocI1C = __sveltets_2_ensureComponent(Icon); new $$_nocI1C({ target: __sveltets_2_any(), props: {    "name":`plus`,"size":12,}});}    
       }
    }else{
      
       { svelteHTML.createElement("div", { "class":`field`,});
         { svelteHTML.createElement("label", {   "class":`label`,"for":`import-account`,});  }
         { svelteHTML.createElement("select", {       "id":`import-account`,"class":`input`,"bind:value":accountId,"onchange":resetSelection,});/*Ωignore_startΩ*/() => accountId = __sveltets_2_any(null);/*Ωignore_endΩ*/
             for(let a of __sveltets_2_ensureArray(accounts)){a.id;
             { svelteHTML.createElement("option", { "value":a.id,});a.label; a.base_url;  }
          }
         }
       }

      
       { svelteHTML.createElement("div", { "class":`field`,});
         { svelteHTML.createElement("span", { "class":`label`,});  }
         { svelteHTML.createElement("div", { "class":`kind-row`,});
           { svelteHTML.createElement("label", {  "class":`kind-opt`,});sourceKind === 'jira';
             { svelteHTML.createElement("input", {       "type":`radio`,"value":`jira`,"onchange":resetSelection,});sourceKind = __sveltets_2_any(null);}
             { const $$_nocI3C = __sveltets_2_ensureComponent(Icon); new $$_nocI3C({ target: __sveltets_2_any(), props: {    "name":`ticket`,"size":13,}});}
            isLinear ? 'Linear issue' : 'Jira issue';
           }
          if(!isLinear){
             { svelteHTML.createElement("label", {  "class":`kind-opt`,});sourceKind === 'confluence';
               { svelteHTML.createElement("input", {       "type":`radio`,"value":`confluence`,"onchange":resetSelection,});sourceKind = __sveltets_2_any(null);}
               { const $$_nocI3C = __sveltets_2_ensureComponent(Icon); new $$_nocI3C({ target: __sveltets_2_any(), props: {    "name":`globe`,"size":13,}});}
               
             }
          }
         }
       }

      
      if(selectedKey){
         { svelteHTML.createElement("div", { "class":`selected-chip`,});
           { svelteHTML.createElement("span", { "class":`selected-label`,});selectedLabel || selectedKey; }
           { svelteHTML.createElement("button", {     "class":`change-btn`,"onclick":() => { selectedKey = ''; selectedLabel = ''; },});
            
           }
         }
      }else{
        
         { svelteHTML.createElement("div", { "class":`field search-field`,});
           { svelteHTML.createElement("span", { "class":`label`,});
            sourceKind === 'jira' ? 'Issue' : 'Page';
           }
           { const $$_hcraeSecruoS1C = __sveltets_2_ensureComponent(SourceSearch); new $$_hcraeSecruoS1C({ target: __sveltets_2_any(), props: {       accountId,sourceKind,"provider":accountProvider,"onpick":(s) => { selectedKey = s.key; selectedLabel = s.label; },}});}
         }
      }

      
       { svelteHTML.createElement("div", { "class":`manual-toggle-row`,});
         { svelteHTML.createElement("button", {         "class":`manual-toggle`,"type":`button`,"aria-expanded":showManual,"onclick":() => (showManual = !showManual),});
           { const $$_nocI2C = __sveltets_2_ensureComponent(Icon); new $$_nocI2C({ target: __sveltets_2_any(), props: {    "name":showManual ? 'chevronDown' : 'chevronRight',"size":12,}});}
           sourceKind === 'jira' ? 'issue key' : 'page ID'; 
         }
       }

      if(showManual){
         { svelteHTML.createElement("div", { "class":`field`,});
           { svelteHTML.createElement("label", {   "class":`label`,"for":`import-key`,});
            sourceKind === 'jira' ? 'Issue key' : 'Page ID or URL';
           }
           { svelteHTML.createElement("input", {              "dir":`auto`,"id":`import-key`,"class":`input`,"bind:value":manualKey,"placeholder":sourceKind === 'jira' ? 'e.g. PROJ-123' : 'e.g. 123456789 or page URL',"spellcheck":`false`,"autocomplete":`off`,});/*Ωignore_startΩ*/() => manualKey = __sveltets_2_any(null);/*Ωignore_endΩ*/}
         }
      }

      
       { svelteHTML.createElement("div", { "class":`field`,});
         { svelteHTML.createElement("label", {   "class":`label`,"for":`import-cwd`,});   { svelteHTML.createElement("span", { "class":`dim`,});  } }
         { const $$_dleiFhtaP1C = __sveltets_2_ensureComponent(PathField); const $$_dleiFhtaP1 = new $$_dleiFhtaP1C({ target: __sveltets_2_any(), props: {  children:() => { return __sveltets_2_any(0); },value:cwd,}});/*Ωignore_startΩ*/() => cwd = __sveltets_2_any(null);/*Ωignore_endΩ*/$$_dleiFhtaP1.$$bindings = 'value';
           { svelteHTML.createElement("input", {              "dir":`ltr`,"id":`import-cwd`,"class":`input`,"bind:value":cwd,"placeholder":`/path/to/repo`,"spellcheck":`false`,"autocomplete":`off`,});/*Ωignore_startΩ*/() => cwd = __sveltets_2_any(null);/*Ωignore_endΩ*/}
         PathField}
       }

      
       { svelteHTML.createElement("label", { "class":`watch-row`,});
         { svelteHTML.createElement("input", {    "type":`checkbox`,"bind:checked":watchEnabled,});/*Ωignore_startΩ*/() => watchEnabled = __sveltets_2_any(null);/*Ωignore_endΩ*/}
         { svelteHTML.createElement("span", {});     }
       }

      if(formError){
         { svelteHTML.createElement("div", { "class":`field-error`,});formError; }
      }
    }
  };return __sveltets_2_any(0)},footer:() => { async ()/*Ωignore_positionΩ*/ => {
     { svelteHTML.createElement("button", {     "class":`btn ghost`,"onclick":onclose,"disabled":submitting,});  }
     { svelteHTML.createElement("button", {       "class":`btn primary`,"onclick":submit,"disabled":submitting || accountsLoading || accounts.length === 0 || !effectiveKey,});
      submitting ? 'Importing…' : 'Import';
     }
  };return __sveltets_2_any(0)},}});/*Ωignore_startΩ*/const {children, footer} = $$_ladoM0.$$prop_def;/*Ωignore_endΩ*/
  

  
 Modal}



};
return { props: {} as any as Props, exports: {}, bindings: __sveltets_$$bindings(''), slots: {}, events: {} }}
const ImportDialog__SvelteComponent_ = __sveltets_2_fn_component($$render());
/*Ωignore_startΩ*/type ImportDialog__SvelteComponent_ = ReturnType<typeof ImportDialog__SvelteComponent_>;
/*Ωignore_endΩ*/export default ImportDialog__SvelteComponent_;