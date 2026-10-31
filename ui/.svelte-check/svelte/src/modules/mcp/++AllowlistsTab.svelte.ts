///<reference types="svelte" />
;// Per-workspace allow/deny entries over this workspace's own servers' tools.
// A `deny` always wins; a tool-less entry covers the whole server; with no
// match the server's `default_tool_access` applies. Edited as a grid, saved in
// one bulk PUT (the server replaces the whole workspace allowlist).

import { tick } from 'svelte';
import Icon from '../../lib/components/Icon.svelte';
import { toastError } from '../../lib/toastError';
import LoadState from '../../lib/components/LoadState.svelte';
import EmptyState from '../../lib/components/EmptyState.svelte';
import { plural } from '../../lib/plural';
import { loadErrorText } from '../../lib/loadError';
import { mcpCpApi } from '../../lib/api/mcp';
import { toasts } from '../../lib/toast.svelte';
import type { McpServerDetail, McpToolAccess } from '../../lib/api/types';
;

  interface Props {
    wsId: string;
    servers: McpServerDetail[];
  };;

  interface Row {
    server_id: string;
    tool_name: string;
    mode: McpToolAccess;
  };function $$render() {

  
  
  
  
  
  
  
  
  
  
  
  
  
  

  let { wsId, servers }: Props = $props();

  let rows = $state<Row[]>([]);
  let loading = $state(false);
  /** Failed load — inline with Retry, never the empty state. */
  let loadError = $state<string | null>(null);
  let saving = $state(false);

  async function load(): Promise<void> {
    loading = true;
    try {
      const entries = await mcpCpApi.cpAllowlist(wsId);
      rows = entries.map((e) => ({
        server_id: e.server_id,
        tool_name: e.tool_name ?? '',
        mode: e.mode,
      }));
      loadError = null;
    } catch (e) {
      loadError = loadErrorText(e);
    } finally {
      loading = false;
    }
  }

  $effect(() => {
    void wsId;
    void load();
  });

  function addRow(): void {
    rows = [...rows, { server_id: servers[0]?.id ?? '', tool_name: '', mode: 'allow' }];
  }
  let grid = $state<HTMLElement | null>(null);
  let addBtn = $state<HTMLButtonElement | null>(null);
  /** "github → create_issue" / "github (whole server)": names a row's controls,
   *  so a screen-reader user knows which entry a Remove button removes. */
  function rowName(r: Row): string {
    const server = servers.find((s) => s.id === r.server_id)?.name ?? 'no server';
    return r.tool_name.trim() ? `${server} → ${r.tool_name.trim()}` : `${server} (whole server)`;
  }
  /** Remove a row, then keep focus in the grid: the row now at that index, the
   *  previous one, or "Add entry" once the list is empty (WCAG 2.4.3 — the
   *  focused button was just destroyed). */
  async function removeRow(i: number): Promise<void> {
    rows = rows.filter((_, idx) => idx !== i);
    await tick();
    const next = Math.min(i, rows.length - 1);
    const target =
      next >= 0 ? grid?.querySelector<HTMLButtonElement>(`[data-remove-row="${next}"]`) : null;
    (target ?? addBtn)?.focus();
  }

  async function save(): Promise<void> {
    const entries = rows
      .filter((r) => r.server_id)
      .map((r) => ({
        server_id: r.server_id,
        tool_name: r.tool_name.trim() || null,
        mode: r.mode,
      }));
    saving = true;
    try {
      await mcpCpApi.cpSetAllowlist(wsId, { entries });
      toasts.success('Allowlist saved', plural(entries.length, 'entry', 'entries'));
      await load();
    } catch (e) {
      toastError('Couldn’t save the allowlist', e);
    } finally {
      saving = false;
    }
  }
;
async () => {

 { svelteHTML.createElement("div", { "class":`al`,});
   { svelteHTML.createElement("div", { "class":`bar`,});
     { svelteHTML.createElement("span", { "class":`hint`,});
        { svelteHTML.createElement("strong", {});  }              
              
     }
     { svelteHTML.createElement("span", { "class":`grow`,}); }
     { const $$_button2 = svelteHTML.createElement("button", {      "class":`btn small`,"onclick":addRow,"disabled":servers.length === 0,});addBtn = $$_button2;
       { const $$_nocI3C = __sveltets_2_ensureComponent(Icon); new $$_nocI3C({ target: __sveltets_2_any(), props: {    "name":`plus`,"size":13,}});}  
     }
     { svelteHTML.createElement("button", {     "class":`btn primary small`,"onclick":() => void save(),"disabled":saving,});
      saving ? 'Saving…' : 'Save';
     }
   }

  if(servers.length === 0){
     { svelteHTML.createElement("p", { "class":`muted pad`,});          }
  }else{
     { const $$_etatSdaoL1C = __sveltets_2_ensureComponent(LoadState); const $$_etatSdaoL1 = new $$_etatSdaoL1C({ target: __sveltets_2_any(), props: {        children:() => { return __sveltets_2_any(0); },"what":`the allowlist`,loading,"error":loadError,"empty":rows.length === 0,"onretry":() => void load(),emptyView:() => { async ()/*Ωignore_positionΩ*/ => {
         { const $$_etatSytpmE0C = __sveltets_2_ensureComponent(EmptyState); new $$_etatSytpmE0C({ target: __sveltets_2_any(), props: {              "icon":`eye`,"title":`No allowlist entries yet`,"body":`Without any, each server’s default tool access applies.`,"actionLabel":`Add an entry`,"actionIcon":`plus`,"actionKind":`secondary`,"onaction":addRow,}});}
      };return __sveltets_2_any(0)},}});/*Ωignore_startΩ*/const {emptyView} = $$_etatSdaoL1.$$prop_def;/*Ωignore_endΩ*/
      
     { const $$_div2 = svelteHTML.createElement("div", {  "class":`grid`,});grid = $$_div2;
       { svelteHTML.createElement("div", { "class":`thead`,});
         { svelteHTML.createElement("span", {});  }
         { svelteHTML.createElement("span", {});  { svelteHTML.createElement("em", {});    } }
         { svelteHTML.createElement("span", {});  }
         { svelteHTML.createElement("span", {}); }
       }
          for(let r of __sveltets_2_ensureArray(rows)){let i = 1;i;
         { svelteHTML.createElement("div", { "class":`arow`,});
           { svelteHTML.createElement("select", {   "bind:value":r.server_id,"aria-label":`Server`,});/*Ωignore_startΩ*/() => r.server_id = __sveltets_2_any(null);/*Ωignore_endΩ*/
               for(let s of __sveltets_2_ensureArray(servers)){s.id;
               { svelteHTML.createElement("option", { "value":s.id,});s.name; }
            }
           }
           { svelteHTML.createElement("input", {          "dir":`ltr`,"bind:value":r.tool_name,"placeholder":`(whole server)`,"class":`mono`,"aria-label":`Tool (blank = whole server)`,});/*Ωignore_startΩ*/() => r.tool_name = __sveltets_2_any(null);/*Ωignore_endΩ*/}
           { svelteHTML.createElement("select", {   "bind:value":r.mode,"aria-label":`Mode`,});/*Ωignore_startΩ*/() => r.mode = __sveltets_2_any(null);/*Ωignore_endΩ*/
             { svelteHTML.createElement("option", { "value":`allow`,});  }
             { svelteHTML.createElement("option", { "value":`deny`,});  }
           }
           { svelteHTML.createElement("button", {         "class":`icon danger`,...__sveltets_2_empty({"data-remove-row":i}),"onclick":() => void removeRow(i),"title":`Remove entry`,"aria-label":`Remove entry ${rowName(r)}`,});
             { const $$_nocI5C = __sveltets_2_ensureComponent(Icon); new $$_nocI5C({ target: __sveltets_2_any(), props: {    "name":`trash`,"size":14,}});}
           }
         }
      }
     }
     LoadState}
  }
 }


};
return { props: {} as any as Props, exports: {}, bindings: __sveltets_$$bindings(''), slots: {}, events: {} }}
const AllowlistsTab__SvelteComponent_ = __sveltets_2_fn_component($$render());
/*Ωignore_startΩ*/type AllowlistsTab__SvelteComponent_ = ReturnType<typeof AllowlistsTab__SvelteComponent_>;
/*Ωignore_endΩ*/export default AllowlistsTab__SvelteComponent_;