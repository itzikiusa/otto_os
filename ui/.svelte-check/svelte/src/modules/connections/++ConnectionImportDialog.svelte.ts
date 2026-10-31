///<reference types="svelte" />
;
import { announce } from '../../lib/announce.svelte';
import { loadErrorText } from '../../lib/loadError';
import Badge from '../../lib/components/Badge.svelte';
import { plural } from '../../lib/plural';
// Import connections from another DB tool. No file picker, no path config: the
// daemon reads each tool's config from its own standard location. Three steps —
//   1. pick a tool (the daemon already told us which are present + how many),
//   2. scan it and preview the parsed connections (supported rows checked,
//      unsupported rows greyed with the reason),
//   3. create the kept ones (passwords are never imported — the user sets them
//      after, exactly like a hand-made connection).
import Modal from '../../lib/components/Modal.svelte';
import { toastError } from '../../lib/toastError';
import Icon, { type IconName } from '../../lib/components/Icon.svelte';
import { toasts } from '../../lib/toast.svelte';
import { api, importSources, importScan, importCreate } from '../../lib/api/client';
import type {
    Connection,
    ConnectionKind,
    ImportSource,
    SourceStatus,
    ParsedConnection,
    ImportCreateItem,
  } from '../../lib/api/types';
;

  interface Props {
    /** Workspace the import endpoints are scoped to (same one the page uses). */
    wsId: string;
    onclose: () => void;
    /** Called after a successful create so the page can refresh its list. */
    onimported: () => void;
  };;

  type Step = 'pick' | 'preview';;function $$render() {

  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  

  let { wsId, onclose, onimported }: Props = $props();

  // Same kind→icon mapping the connection list rows use, so a row reads the same.
  const kindIcons: Record<ConnectionKind, IconName> = {
    ssh: 'key',
    mysql: 'db',
    postgres: 'db',
    redis: 'zap',
    mongodb: 'db',
    clickhouse: 'db',
    custom: 'terminal',
  };

  let step = $state<Step>('pick');

  // Step 1 — tool list.
  let sources = $state<SourceStatus[]>([]);
  let loadingSources = $state(true);
  // A failed load must not read as "no tools found" — shown inline with Retry.
  let sourcesError = $state('');

  // Step 2 — scan of one tool.
  let activeSource = $state<ImportSource | null>(null);
  let scanning = $state(false);
  let scanPath = $state<string | null>(null);
  let warnings = $state<string[]>([]);
  let rows = $state<ParsedConnection[]>([]);
  // Per-row keep flag (index-aligned with `rows`). Unsupported rows are never set.
  let keep = $state<boolean[]>([]);
  let existing = $state<Connection[]>([]);
  let actions = $state<('create' | 'update' | 'skip')[]>([]);
  let targets = $state<string[]>([]);

  // Step 3 — create in flight.
  let creating = $state(false);

  const activeLabel = $derived(
    sources.find((s) => s.source === activeSource)?.label ?? activeSource ?? '',
  );
  const selectableCount = $derived(rows.filter((r) => r.supported).length);
  const selectedCount = $derived(keep.filter(Boolean).length);
  const allSelected = $derived(selectableCount > 0 && selectedCount === selectableCount);
  // Selected rows that will actually do something (a kept row set to "Skip"
  // imports nothing, so it isn't counted on the Import button).
  const actionableCount = $derived(
    rows.filter((r, i) => r.supported && keep[i] && (actions[i] ?? 'create') !== 'skip').length,
  );
  // An "Update existing" row with no target picked can't be sent.
  const missingTarget = $derived(
    rows.some((r, i) => r.supported && keep[i] && actions[i] === 'update' && !targets[i]),
  );

  $effect(() => {
    void loadSources();
  });

  async function loadSources(): Promise<void> {
    loadingSources = true;
    sourcesError = '';
    try {
      sources = await importSources(wsId);
    } catch (e) {
      sourcesError = loadErrorText(e);
      sources = [];
    } finally {
      loadingSources = false;
    }
  }

  async function pick(s: SourceStatus): Promise<void> {
    if (!s.present || scanning) return;
    activeSource = s.source;
    step = 'preview';
    scanning = true;
    rows = [];
    keep = [];
    warnings = [];
    scanPath = s.path ?? null;
    try {
      const [res, saved] = await Promise.all([importScan(wsId, s.source), api.get<Connection[]>(`/workspaces/${wsId}/connections`)]);
      existing = saved;
      targets = res.connections.map(c => { const matches = saved.filter(e => e.name === c.name && e.kind === c.kind); return matches.length === 1 ? matches[0].id : ''; });
      actions = res.connections.map((c, i) => targets[i] || !c.supported ? 'skip' : 'create');
      rows = res.connections;
      scanPath = res.path ?? s.path ?? null;
      warnings = res.warnings;
      // Supported rows are kept by default; unsupported can never be selected.
      keep = res.connections.map((c) => c.supported);
    } catch (e) {
      toastError('Couldn’t scan for connections', e);
      // Drop back to the picker so the user can retry / choose another tool.
      step = 'pick';
      activeSource = null;
    } finally {
      scanning = false;
    }
  }

  function backToPick(): void {
    if (creating) return;
    step = 'pick';
    activeSource = null;
    rows = [];
    keep = [];
    warnings = [];
    scanPath = null;
  }

  function toggleRow(i: number): void {
    if (!rows[i]?.supported) return;
    keep[i] = !keep[i];
  }

  function selectAll(on: boolean): void {
    keep = rows.map((c, i) => (c.supported ? on : (keep[i] ?? false)));
  }

  // Best-effort one-line summary of a parsed connection's params (host / db),
  // falling back to the name when nothing obvious is there.
  function summarize(c: ParsedConnection): string {
    const p = (c.params ?? {}) as Record<string, unknown>;
    if (c.kind === 'mongodb') return String(p.conn_string ?? '');
    if (c.kind === 'custom') return String(p.command_template ?? '');
    const host = p.host !== undefined && p.host !== '' ? String(p.host) : '';
    if (!host) return '';
    const port = p.port !== undefined && p.port !== '' ? `:${p.port}` : '';
    const db = p.db !== undefined && p.db !== '' ? ` / ${p.db}` : '';
    const user = p.user !== undefined && p.user !== '' ? `${p.user}@` : '';
    return `${user}${host}${port}${db}`;
  }

  async function create(): Promise<void> {
    if (creating) return;
    const picked: ImportCreateItem[] = rows.flatMap((c, i) => c.supported && keep[i] && c.kind ? [{
      name: c.name, kind: c.kind, params: c.params, action: actions[i] ?? 'create',
      ...(actions[i] === 'update' ? { target_id: targets[i] } : {}),
    }] : []);
    if (picked.length === 0) {
      toasts.info('Nothing selected', 'Pick at least one connection to import.');
      return;
    }
    creating = true;
    try {
      const res = await importCreate(wsId, { connections: picked, section_id: null });
      const made = res.created.length + res.updated.length;
      const failed = res.failed.length;
      if (made > 0) {
        toasts.success(
          `Imported ${made}`,
          failed > 0
            ? `${failed} failed — set passwords on imported connections before connecting.`
            : `${res.created.length} created, ${res.updated.length} updated, ${res.skipped.length} skipped. Existing passwords are retained on updates.`,
        );
      }
      if (failed > 0) {
        // Surface the individual failures so they aren't silently lost.
        const detail = res.failed.map((f) => `${f.name}: ${f.error}`).join('\n');
        toasts.error(`Couldn’t import ${plural(failed, 'connection')}`, detail);
      }
      if (made === 0 && failed === 0) {
        toasts.info('Nothing imported');
      }
      onimported();
      onclose();
    } catch (e) {
      toastError('Couldn’t import the connections', e);
    } finally {
      creating = false;
    }
  }

  function close(): void {
    if (creating) return;
    onclose();
  }
  // Spoken at the state edge through the shell's always-mounted announcer: a
  // live region inserted together with its text is not read (WebKit/VoiceOver).
  $effect(() => { if (loadingSources) announce('Looking for installed tools…'); });
  $effect(() => { if (sourcesError) announce(`Couldn’t look for installed tools. ${sourcesError}`, { assertive: true }); });
  $effect(() => { if (scanning) announce(`Scanning ${activeLabel}…`); });
;
async () => {

 { const $$_ladoM0C = __sveltets_2_ensureComponent(Modal); const $$_ladoM0 = new $$_ladoM0C({ target: __sveltets_2_any(), props: {     children:() => { return __sveltets_2_any(0); },"title":`Import connections`,"width":560,"onclose":close,footer:() => { async ()/*Ωignore_positionΩ*/ => {
    if(step === 'pick'){
       { svelteHTML.createElement("button", {   "class":`btn`,"onclick":close,});  }
    }else{
       { svelteHTML.createElement("button", {     "class":`btn`,"onclick":close,"disabled":creating,});  }
       { svelteHTML.createElement("button", {         "class":`btn primary`,"onclick":() => void create(),"disabled":creating || scanning || actionableCount === 0 || missingTarget,"title":missingTarget
          ? 'Choose which connection to update for every "Update existing" row'
          : actionableCount === 0 && !scanning
            ? 'Select at least one connection to create or update'
            : undefined,});
        creating ? 'Importing…' : `Import ${actionableCount} selected`;
       }
    }
  };return __sveltets_2_any(0)},}});/*Ωignore_startΩ*/const {footer} = $$_ladoM0.$$prop_def;/*Ωignore_endΩ*/
  if(step === 'pick'){
     { svelteHTML.createElement("div", { "class":`imp`,});
       { svelteHTML.createElement("p", { "class":`imp-hint`,});
                     
                      
            
       }

      if(loadingSources){
         { svelteHTML.createElement("div", { "class":`imp-loading`,});
           { svelteHTML.createElement("div", {     "class":`indeterminate`,"role":`progressbar`,"aria-label":`Looking for installed tools`,}); }
           { svelteHTML.createElement("span", { "class":`imp-loading-text`,});    }
         }
      } else if (sourcesError){
         { svelteHTML.createElement("div", { "class":`imp-error`,});
           { svelteHTML.createElement("span", {});     sourcesError; }
           { svelteHTML.createElement("button", {   "class":`btn small`,"onclick":() => void loadSources(),});  }
         }
      }else{
         { svelteHTML.createElement("div", { "class":`tool-grid`,});
             for(let s of __sveltets_2_ensureArray(sources)){s.source;
             { svelteHTML.createElement("button", {          "class":`tool`,"disabled":!s.present,"onclick":() => void pick(s),"aria-label":s.present
                ? `${s.label} — ${s.count ?? 0} connections found`
                : `${s.label} — not found`,});s.present;
               { svelteHTML.createElement("span", { "class":`tool-icon`,}); { const $$_nocI5C = __sveltets_2_ensureComponent(Icon); new $$_nocI5C({ target: __sveltets_2_any(), props: {    "name":`plug`,"size":16,}});} }
               { svelteHTML.createElement("span", { "class":`tool-body`,});
                 { svelteHTML.createElement("span", { "class":`tool-name`,});s.label; }
                 { svelteHTML.createElement("span", {  "class":`tool-sub`,});s.present;
                  if(s.present){
                    plural(s.count ?? 0, 'connection'); }else{ }
                 }
               }
              if(s.present){
                 { svelteHTML.createElement("span", { "class":`tool-caret`,}); { const $$_nocI5C = __sveltets_2_ensureComponent(Icon); new $$_nocI5C({ target: __sveltets_2_any(), props: {    "name":`chevronRight`,"size":13,}});} }
              }
             }
          }
         }
        if(sources.every((s) => !s.present)){
           { svelteHTML.createElement("p", { "class":`imp-empty`,});
                          
                  
           }
        }
      }
     }
  }else{
     { svelteHTML.createElement("div", { "class":`imp`,});
       { svelteHTML.createElement("div", { "class":`prev-head`,});
         { svelteHTML.createElement("button", {     "class":`btn small`,"onclick":backToPick,"disabled":creating,});
           { const $$_nocI4C = __sveltets_2_ensureComponent(Icon); new $$_nocI4C({ target: __sveltets_2_any(), props: {    "name":`chevronLeft`,"size":12,}});} 
         }
         { svelteHTML.createElement("span", { "class":`prev-title`,});activeLabel; }
        if(scanPath){
           { svelteHTML.createElement("span", {   "class":`prev-path mono ellipsis`,"title":scanPath,});  scanPath; }
        }
       }

      if(scanning){
         { svelteHTML.createElement("div", { "class":`imp-loading`,});
           { svelteHTML.createElement("div", {     "class":`indeterminate`,"role":`progressbar`,"aria-label":`Scanning ${activeLabel}`,}); }
           { svelteHTML.createElement("span", { "class":`imp-loading-text`,}); activeLabel;  }
         }
      }else{
        if(warnings.length > 0){
           { svelteHTML.createElement("div", { "class":`prev-warn`,});
                for(let w of __sveltets_2_ensureArray(warnings)){let i = 1;i;
               { svelteHTML.createElement("div", { "class":`prev-warn-line`,}); { const $$_nocI4C = __sveltets_2_ensureComponent(Icon); new $$_nocI4C({ target: __sveltets_2_any(), props: {    "name":`info`,"size":12,}});} w; }
            }
           }
        }

        if(rows.length === 0){
           { svelteHTML.createElement("p", { "class":`imp-empty`,});     activeLabel;  }
        }else{
           { svelteHTML.createElement("div", { "class":`prev-toolbar`,});
             { svelteHTML.createElement("span", { "class":`prev-count`,});
              selectedCount;  selectableCount; 
             }
             { svelteHTML.createElement("span", { "class":`grow`,}); }
             { svelteHTML.createElement("button", {       "class":`link-btn`,"disabled":selectableCount === 0 || allSelected,"onclick":() => selectAll(true),});
               
             }
             { svelteHTML.createElement("span", { "class":`dot-sep`,});  }
             { svelteHTML.createElement("button", {       "class":`link-btn`,"disabled":selectedCount === 0,"onclick":() => selectAll(false),});
              
             }
           }

           { svelteHTML.createElement("div", {   "class":`rows`,"role":`list`,});
                for(let c of __sveltets_2_ensureArray(rows)){let i = 1;c.name + i;
               { svelteHTML.createElement("div", {    "class":`import-row`,"role":`listitem`,});!c.supported;
                 { svelteHTML.createElement("input", {             "type":`checkbox`,"class":`row-check`,"checked":keep[i] ?? false,"disabled":!c.supported,"onchange":() => toggleRow(i),"aria-label":`Import ${c.name}`,});}
                 { svelteHTML.createElement("span", {   "class":`row-kind`,"title":c.kind ?? 'unsupported',});
                   { const $$_nocI5C = __sveltets_2_ensureComponent(Icon); new $$_nocI5C({ target: __sveltets_2_any(), props: {    "name":c.kind ? kindIcons[c.kind] : 'plug',"size":13,}});}
                 }
                 { svelteHTML.createElement("span", { "class":`row-main`,});
                   { svelteHTML.createElement("span", { "class":`row-name ellipsis`,});c.name; }
                  if(summarize(c)){
                     { svelteHTML.createElement("span", { "class":`row-desc mono ellipsis`,});summarize(c); }
                  }
                 }
                 { svelteHTML.createElement("span", { "class":`grow`,}); }
                if(c.supported){
                   { svelteHTML.createElement("select", {     "aria-label":`Action for ${c.name}`,"bind:value":actions[i],"disabled":!keep[i],});/*Ωignore_startΩ*/() => actions[i] = __sveltets_2_any(null);/*Ωignore_endΩ*/
                     { svelteHTML.createElement("option", { "value":`create`,});  } { svelteHTML.createElement("option", { "value":`skip`,});  }
                     { svelteHTML.createElement("option", {   "value":`update`,"disabled":!existing.some(e => e.kind === c.kind),});  }
                   }
                  if(actions[i] === 'update'){
                     { svelteHTML.createElement("select", {    "aria-label":`Update target for ${c.name}`,"bind:value":targets[i],"required":true,});/*Ωignore_startΩ*/() => targets[i] = __sveltets_2_any(null);/*Ωignore_endΩ*/
                       { svelteHTML.createElement("option", {"value":"","disabled":true,});  }
                         for(let target of __sveltets_2_ensureArray(existing.filter(e => e.kind === c.kind))){target.id; { svelteHTML.createElement("option", { "value":target.id,});target.name; }}
                     }
                  }
                  if(c.kind){ { const $$_egdaB4C = __sveltets_2_ensureComponent(Badge); new $$_egdaB4C({ target: __sveltets_2_any(), props: {    "variant":`outline`,"label":c.kind,}});}}
                  if(c.needs_password){
                     { const $$_egdaB4C = __sveltets_2_ensureComponent(Badge); new $$_egdaB4C({ target: __sveltets_2_any(), props: {      "tone":`warn`,"label":`Needs password`,"title":`No password was imported — set it before connecting`,}});}
                  }
                }else{
                   { const $$_egdaB4C = __sveltets_2_ensureComponent(Badge); new $$_egdaB4C({ target: __sveltets_2_any(), props: {    "label":c.note ?? 'Not supported',"title":c.note ?? 'Not supported',}});}
                }
               }
            }
           }

           { svelteHTML.createElement("p", { "class":`imp-note`,});
             { const $$_nocI3C = __sveltets_2_ensureComponent(Icon); new $$_nocI3C({ target: __sveltets_2_any(), props: {    "name":`key`,"size":12,}});}         
              
           }
        }
      }
     }
  }

  
 Modal}


};
return { props: {} as any as Props, exports: {}, bindings: __sveltets_$$bindings(''), slots: {}, events: {} }}
const ConnectionImportDialog__SvelteComponent_ = __sveltets_2_fn_component($$render());
/*Ωignore_startΩ*/type ConnectionImportDialog__SvelteComponent_ = ReturnType<typeof ConnectionImportDialog__SvelteComponent_>;
/*Ωignore_endΩ*/export default ConnectionImportDialog__SvelteComponent_;