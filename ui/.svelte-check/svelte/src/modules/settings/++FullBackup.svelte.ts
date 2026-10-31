///<reference types="svelte" />
;
import { announce } from '../../lib/announce.svelte';
import { onDestroy } from 'svelte';
import RestorePreviewPanel from './RestorePreviewPanel.svelte';
import { api } from '../../lib/api/client';
import type { StateArchive, RestoreConflictPolicy, RestorePreview, RestoreResult } from '../../lib/api/types';
import { downloadText } from '../../lib/components/exporters';
import Icon from '../../lib/components/Icon.svelte';
function $$render() {

  
  
  
  
  
  
  

  let exporting = $state(false);
  let busy = $state(false);
  let error = $state('');
  let archive: StateArchive | null = $state(null);
  let preview: RestorePreview | null = $state(null);
  let result: RestoreResult | null = $state(null);
  let filename = $state('');
  let conflicts: RestoreConflictPolicy = $state('skip_existing');
  let reviewed = $state(false);
  let downloaded: { records: number; files: number; excluded: string[]; reconnect: string[] } | null = $state(null);
  let generation = 0;
  let fileEl: HTMLInputElement | null = $state(null);
  onDestroy(() => { generation++; });
  // Inline errors say which step failed, then the daemon's reason.
  const msg = (e: unknown): string => (e instanceof Error ? e.message : String(e));

  async function exportArchive() {
    const seq = generation;
    exporting = true;
    error = '';
    try {
      const data = await api.get<StateArchive>('/state/archive');
      if (seq !== generation) return;
      downloadText(JSON.stringify(data), `otto-data-${new Date().toISOString().slice(0, 10)}.json`, 'application/json');
      downloaded = {
        records: Object.values(data.records).reduce((sum, rows) => sum + rows.length, 0),
        files: data.files.length, excluded: data.excluded, reconnect: data.reconnect,
      };
    } catch (e) { if (seq === generation) error = `Couldn’t prepare the data archive. ${msg(e)}`; }
    finally { if (seq === generation) exporting = false; }
  }

  async function readArchive(event: Event) {
    const input = event.target as HTMLInputElement;
    const file = input.files?.[0];
    input.value = '';
    if (!file) return;
    const seq = ++generation;
    archive = null; preview = null; result = null; reviewed = false; error = ''; filename = file.name;
    busy = true;
    try {
      if (file.size > 256 * 1024 * 1024) throw new Error('This archive exceeds the 256 MiB import limit.');
      const data: unknown = JSON.parse(await file.text());
      if (!data || typeof data !== 'object' || !('archive_format' in data) || data.archive_format !== 2) {
        throw new Error('Choose an Otto data archive (format 2). Older settings backups use the settings restore section below.');
      }
      if (seq === generation) archive = data as StateArchive;
    } catch (e) { if (seq === generation) error = `Couldn’t read “${file.name}”. ${msg(e)}`; }
    finally { if (seq === generation) busy = false; }
  }

  function changePolicy() { preview = null; result = null; reviewed = false; }

  async function inspectArchive() {
    if (!archive) return;
    const seq = generation;
    busy = true; error = ''; preview = null; result = null; reviewed = false;
    try {
      const data = await api.post<RestorePreview>('/state/archive/preview', { archive, conflicts });
      if (seq === generation) preview = data;
    } catch (e) { if (seq === generation) error = `Couldn’t preview the restore. ${msg(e)}`; }
    finally { if (seq === generation) busy = false; }
  }

  async function restoreArchive() {
    if (!archive || !preview?.can_restore || !reviewed) return;
    const seq = generation;
    busy = true; error = '';
    try {
      const data = await api.post<RestoreResult>('/state/archive/restore', {
        archive, conflicts, preview_token: preview.preview_token, confirm: true,
      });
      if (seq !== generation) return;
      result = data; preview = null; reviewed = false;
    } catch (e) {
      if (seq === generation) {
        error = `Couldn’t restore the archive. ${msg(e)} Preview it again to retry.`;
        preview = null; reviewed = false; // A failed/stale preview must be checked again.
      }
    } finally { if (seq === generation) busy = false; }
  }

  // a11y-6-01: announce at the state edge — a status element inserted
  // together with its text is not read by VoiceOver/WebKit.
  $effect(() => { if (downloaded) announce(`Downloaded ${downloaded.records} records and ${downloaded.files} files.`); });
  $effect(() => { if (result) announce(`Restored ${result.records_inserted} records and ${result.files_restored} files.`); });
;
async () => {

 { svelteHTML.createElement("section", {   "class":`full-backup`,"aria-label":`Otto data backup`,});
   { svelteHTML.createElement("h2", { "class":`card-title`,});   }
   { svelteHTML.createElement("p", {});          
              }
   { svelteHTML.createElement("p", { "class":`dim`,});                       
                   }
   { svelteHTML.createElement("div", { "class":`controls`,}); { svelteHTML.createElement("button", {     "class":`btn primary`,"disabled":exporting || busy,"onclick":exportArchive,}); { const $$_nocI3C = __sveltets_2_ensureComponent(Icon); new $$_nocI3C({ target: __sveltets_2_any(), props: {    "name":`download`,"size":13,}});} exporting ? 'Preparing archive…' : 'Download data archive'; } }
  if(downloaded){
     { svelteHTML.createElement("div", { "class":`notice`,}); downloaded.records;   downloaded.files;  }
    if(downloaded.excluded.length){ { svelteHTML.createElement("details", {}); { svelteHTML.createElement("summary", {});    downloaded.excluded.length;  } { svelteHTML.createElement("ul", {});  for(let item of __sveltets_2_ensureArray(downloaded.excluded)){ { svelteHTML.createElement("li", {});item; }} } }}
    if(downloaded.reconnect.length){ { svelteHTML.createElement("details", {}); { svelteHTML.createElement("summary", {});   downloaded.reconnect.length;  } { svelteHTML.createElement("ul", {});  for(let item of __sveltets_2_ensureArray(downloaded.reconnect)){ { svelteHTML.createElement("li", {});item; }} } }}
  }

   { svelteHTML.createElement("h3", { "class":`sub-title`,});   }
   { svelteHTML.createElement("p", {});              
                }
   { svelteHTML.createElement("div", { "class":`controls`,});
    
     { svelteHTML.createElement("button", {     "class":`btn`,"disabled":busy || exporting,"onclick":() => fileEl?.click(),}); { const $$_nocI3C = __sveltets_2_ensureComponent(Icon); new $$_nocI3C({ target: __sveltets_2_any(), props: {    "name":`folder`,"size":13,}});}    }
     { const $$_input2 = svelteHTML.createElement("input", {               "type":`file`,"class":`file-input`,"accept":`.json,application/json`,"disabled":busy || exporting,"onchange":readArchive,"tabindex":-1,"aria-hidden":`true`,});fileEl = $$_input2;}
    if(filename){ { svelteHTML.createElement("span", {   "class":`filename`,"title":filename,});filename; }}
   }
  if(archive){
     { svelteHTML.createElement("div", { "class":`controls`,});
       { svelteHTML.createElement("label", { "class":`policy`,});    
         { svelteHTML.createElement("select", {       "class":`input`,"bind:value":conflicts,"disabled":busy,"onchange":changePolicy,});/*Ωignore_startΩ*/() => conflicts = __sveltets_2_any(null);/*Ωignore_endΩ*/
           { svelteHTML.createElement("option", { "value":`skip_existing`,});      }
           { svelteHTML.createElement("option", { "value":`abort`,});      }
         }
       }
       { svelteHTML.createElement("button", {     "class":`btn`,"disabled":busy,"onclick":inspectArchive,});busy ? 'Working…' : 'Preview restore'; }
     }
  }
  if(preview){
     { const $$_lenaPweiverPerotseR1C = __sveltets_2_ensureComponent(RestorePreviewPanel); const $$_lenaPweiverPerotseR1 = new $$_lenaPweiverPerotseR1C({ target: __sveltets_2_any(), props: {    preview,busy,reviewed,"onrestore":restoreArchive,}});/*Ωignore_startΩ*/() => reviewed = __sveltets_2_any(null);/*Ωignore_endΩ*/$$_lenaPweiverPerotseR1.$$bindings = 'reviewed';}
  }
  if(result){
     { svelteHTML.createElement("div", { "class":`notice`,}); result.records_inserted;   result.files_restored; 
       result.records_skipped;    result.files_skipped;   }
    if(result.reconnect.length){ { svelteHTML.createElement("ul", {});  for(let item of __sveltets_2_ensureArray(result.reconnect)){ { svelteHTML.createElement("li", {});item; }} }}
  }
  if(error){ { svelteHTML.createElement("p", {   "class":`error`,"role":`alert`,});error; }}
 }


};
return { props: {} as Record<string, never>, exports: {}, bindings: __sveltets_$$bindings(''), slots: {}, events: {} }}
const FullBackup__SvelteComponent_ = __sveltets_2_fn_component($$render());
/*Ωignore_startΩ*/type FullBackup__SvelteComponent_ = ReturnType<typeof FullBackup__SvelteComponent_>;
/*Ωignore_endΩ*/export default FullBackup__SvelteComponent_;