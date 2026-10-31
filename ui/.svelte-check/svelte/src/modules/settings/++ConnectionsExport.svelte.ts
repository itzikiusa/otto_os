///<reference types="svelte" />
;
import { announce, focusAppContent } from '../../lib/announce.svelte';
import { onMount, onDestroy } from 'svelte';
import Skeleton from '../../lib/components/Skeleton.svelte';
import LoadState from '../../lib/components/LoadState.svelte';
import { api } from '../../lib/api/client';
import type { ConnectionExportFormat, ConnectionExportResult } from '../../lib/api/types';
import { downloadText } from '../../lib/components/exporters';
function $$render() {

  
  
  
  
  
  
  

  let formats: ConnectionExportFormat[] = $state([]);
  let workspaces: { id: string; name: string }[] = $state([]);
  let format = $state('json');
  let scope: 'all' | 'workspaces' = $state('all');
  let selectedIds: string[] = $state([]);
  let includePasswords = $state(false);
  let loading = $state(true);
  let busy = $state(false);
  let error = $state('');
  let result: ConnectionExportResult | null = $state(null);
  let generation = 0;
  /** Prefix for the label↔select ids. */
  const uid = $props.id();
  const selectedFormat = $derived(formats.find(item => item.id === format));
  onMount(() => { void load(); });
  onDestroy(() => { generation++; result = null; });

  async function load() {
    const seq = generation;
    loading = true; error = '';
    try {
      const [available, spaces] = await Promise.all([
        api.get<{ formats: ConnectionExportFormat[] }>('/state/connections/export/formats'),
        api.get<{ id: string; name: string }[]>('/workspaces'),
      ]);
      if (seq !== generation) return;
      formats = available.formats; workspaces = spaces;
      if (!formats.some(item => item.id === format)) format = formats[0]?.id ?? '';
    } catch (e) { if (seq === generation) error = e instanceof Error ? e.message : String(e); }
    finally { if (seq === generation) loading = false; }
  }
  function clearResult() { result = null; error = ''; }
  function toggleWorkspace(id: string) { selectedIds = selectedIds.includes(id) ? selectedIds.filter(value => value !== id) : [...selectedIds, id]; clearResult(); }
  async function prepare() {
    const seq = generation;
    busy = true; result = null; error = '';
    try {
      const exported = await api.post<ConnectionExportResult>('/state/connections/export', {
        format, scope, workspace_ids: scope === 'workspaces' ? selectedIds : undefined,
        include_passwords: includePasswords,
      });
      if (seq === generation) result = exported;
    } catch (e) { if (seq === generation) error = e instanceof Error ? e.message : String(e); }
    finally { if (seq === generation) busy = false; }
  }

  // a11y-6-01: announce at the state edge — a status element inserted
  // together with its text is not read by VoiceOver/WebKit.
  $effect(() => {
    if (result) announce(`${result.exported_connections} of ${result.total_connections} connections exported${result.contains_passwords ? ' with credentials' : ' without passwords'}.`);
  });
;
async () => {

 { svelteHTML.createElement("section", {   "class":`connections-export`,"aria-label":`Connection export`,});
   { svelteHTML.createElement("h2", { "class":`card-title`,});  }
   { svelteHTML.createElement("p", {});             
                 }
  if(loading){ { const $$_notelekS1C = __sveltets_2_ensureComponent(Skeleton); new $$_notelekS1C({ target: __sveltets_2_any(), props: {      "rows":2,"height":28,"label":`export formats`,}});}} else if (formats.length){
     { svelteHTML.createElement("div", { "class":`controls`,});
       { svelteHTML.createElement("div", { "class":`field`,}); { svelteHTML.createElement("label", { "for":`${uid}-format`,});  } { svelteHTML.createElement("select", {         "id":`${uid}-format`,"class":`input`,"bind:value":format,"disabled":busy,"onchange":clearResult,});/*Ωignore_startΩ*/() => format = __sveltets_2_any(null);/*Ωignore_endΩ*/  for(let item of __sveltets_2_ensureArray(formats)){ { svelteHTML.createElement("option", { "value":item.id,});item.label; }} } }
       { svelteHTML.createElement("div", { "class":`field`,}); { svelteHTML.createElement("label", { "for":`${uid}-scope`,});   } { svelteHTML.createElement("select", {         "id":`${uid}-scope`,"class":`input`,"bind:value":scope,"disabled":busy,"onchange":clearResult,});/*Ωignore_startΩ*/() => scope = __sveltets_2_any(null);/*Ωignore_endΩ*/ { svelteHTML.createElement("option", { "value":`all`,});  } { svelteHTML.createElement("option", { "value":`workspaces`,});  } } }
     }
    if(scope === 'workspaces'){
       { svelteHTML.createElement("p", { "class":`dim`,});        }
       { svelteHTML.createElement("div", {     "class":`workspaces`,"role":`group`,"aria-label":`Export workspaces`,});  for(let workspace of __sveltets_2_ensureArray(workspaces)){ { svelteHTML.createElement("label", {}); { svelteHTML.createElement("input", {        "type":`checkbox`,"checked":selectedIds.includes(workspace.id),"disabled":busy,"onchange":() => toggleWorkspace(workspace.id),});}workspace.name; }} { svelteHTML.createElement("p", {});   } }
    }
    if(selectedFormat){ { svelteHTML.createElement("p", { "class":`dim`,});selectedFormat.description; }}
     { svelteHTML.createElement("label", { "class":`password-option`,}); { svelteHTML.createElement("input", {        "type":`checkbox`,"bind:checked":includePasswords,"disabled":busy,"onchange":clearResult,});/*Ωignore_startΩ*/() => includePasswords = __sveltets_2_any(null);/*Ωignore_endΩ*/}     }
    if(includePasswords){
       { svelteHTML.createElement("p", { "class":`password-note`,});             }
      if(selectedFormat?.password_support === 'sidecar'){ { svelteHTML.createElement("p", {});              }}
    }else{ { svelteHTML.createElement("p", { "class":`dim`,});           }}
     { svelteHTML.createElement("button", {     "class":`btn`,"disabled":busy || !format || (scope === 'workspaces' && selectedIds.length === 0),"onclick":prepare,});busy ? 'Preparing export…' : includePasswords ? 'Prepare export with passwords' : 'Prepare export'; }
  }
  if(result){
     { svelteHTML.createElement("div", {   "class":`result`,"aria-label":`Prepared connection export`,});
       { svelteHTML.createElement("p", {});result.exported_connections;  result.total_connections;  result.contains_passwords ? ' with credentials' : ' without passwords';  }
       { svelteHTML.createElement("div", { "class":`controls`,});  for(let file of __sveltets_2_ensureArray(result.files)){ { svelteHTML.createElement("button", {   "class":`btn`,"onclick":() => downloadText(file.content, file.name, file.mime),}); file.name; }} }
       { svelteHTML.createElement("p", { "class":`instructions`,});result.import_instructions; }
      if(result.warnings.length){ { svelteHTML.createElement("ul", {});  for(let warning of __sveltets_2_ensureArray(result.warnings)){ { svelteHTML.createElement("li", {});warning; }} }}
      if(result.skipped.length){ { svelteHTML.createElement("details", {}); { svelteHTML.createElement("summary", {});   result.skipped.length;  } { svelteHTML.createElement("ul", {});  for(let skipped of __sveltets_2_ensureArray(result.skipped)){ { svelteHTML.createElement("li", {});skipped.name; skipped.kind; skipped.reason; }} } }}
       { svelteHTML.createElement("button", {   "class":`btn ghost`,"onclick":() => { result = null; void focusAppContent(); },});   }
     }
  }
  if(error && !formats.length){
     { const $$_etatSdaoL1C = __sveltets_2_ensureComponent(LoadState); new $$_etatSdaoL1C({ target: __sveltets_2_any(), props: {       "variant":`compact`,"what":`export formats`,error,"empty":true,"onretry":load,}});}
  } else if (error){
     { svelteHTML.createElement("p", {   "class":`error`,"role":`alert`,}); { svelteHTML.createElement("strong", {});    } error; }
  }
 }


};
return { props: {} as Record<string, never>, exports: {}, bindings: __sveltets_$$bindings(''), slots: {}, events: {} }}
const ConnectionsExport__SvelteComponent_ = __sveltets_2_fn_component($$render());
/*Ωignore_startΩ*/type ConnectionsExport__SvelteComponent_ = ReturnType<typeof ConnectionsExport__SvelteComponent_>;
/*Ωignore_endΩ*/export default ConnectionsExport__SvelteComponent_;